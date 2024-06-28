use std::{array::IntoIter, iter::Map};

use camino::{Utf8Path, Utf8PathBuf};
use color_eyre::eyre::{self, eyre};
use rusqlite::{config::DbConfig, params_from_iter, Connection, OpenFlags, Transaction};

use super::{database_traits::*, sql_schemas::*, sql_statements::*};

/// All the tables stored in the [Database]. Used to determine [Database] function behaviour.
pub enum Table {
    Accounts,
    Credentials,
    FilesData,
}

#[derive(Debug)]
pub struct Database {
    /// SQLite database connection.
    connection: Connection,
}
impl Database {
    /// Open a new connection to the database at the given path.
    pub fn connect<P>(path: P) -> eyre::Result<Self>
    where
        P: AsRef<Utf8Path>,
    {
        let connection = Connection::open_with_flags(
            path.as_ref(),
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_FKEY, true)?;

        // Create tables iff they don't exist
        connection.execute(CREATE_ACCOUNTS, ())?;
        connection.execute(CREATE_CREDENTIALS, ())?;
        connection.execute(CREATE_FILES_DATA, ())?;

        Ok(Self { connection })
    }

    /// Create a new database [Transaction].
    pub fn new_transaction(&mut self) -> eyre::Result<Transaction> {
        Ok(self.connection.transaction()?)
    }

    /// Rollback the [Transaction].
    pub fn rollback_transaction(tx: Transaction) -> eyre::Result<()> {
        Ok(tx.rollback()?)
    }

    /// Commit the [Transaction].
    pub fn commit_transaction(tx: Transaction) -> eyre::Result<()> {
        Ok(tx.commit()?)
    }

    /// Retreive a specific entry based on the given primary key.
    ///
    /// Return [Ok<None>] if no entry with that primary key exists in the database.
    pub fn select_entry<T, U, const N: usize>(
        &self,
        primary_key_arr: [U; N],
    ) -> eyre::Result<Option<T>>
    where
        T: TryFromDatabase + HasSqlStatements,
        U: IntoB64,
    {
        let mut statement = self.connection.prepare(T::sql_select())?;
        let params = Self::get_params_iter(primary_key_arr);

        let query_result = statement.query_row(params_from_iter(params), |row| {
            Ok(T::try_from_database(row))
        });
        match query_result {
            Ok(entry) => Ok(Some(entry?)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(eyre!("{err:?}")),
        }
    }

    // /// Delete a database entry, then execute a given function.
    // ///
    // /// The function in question is typically a modification to a filesystem or something else that
    // /// should be consistent with the database.
    // ///
    // /// Should any errors be encountered whilst executing the function or modifying the database
    // /// itself, all changes made to the database will not be committed.
    // pub fn transaction_delete_old<T, U, const N: usize>(
    //     &mut self,
    //     params: [U; N],
    //     fn_result: eyre::Result<()>,
    // ) -> eyre::Result<()>
    // where
    //     T: HasSqlStatements,
    //     U: IntoB64,
    // {
    //     let savepoint = self.connection.savepoint()?;
    //
    //     Self::delete_entry_at_conn::<T, U, N>(&savepoint, params)?;
    //     // If this function fails, then the database will not be modified.
    //     fn_result?;
    //
    //     savepoint.commit()?;
    //     Ok(())
    // }

    /// Delete a specific entry based on the given primary key.
    pub fn delete_entry<T, U, const N: usize>(&self, primary_key_arr: [U; N]) -> eyre::Result<()>
    where
        T: HasSqlStatements,
        U: IntoB64,
    {
        let mut statement = self.connection.prepare(T::sql_delete())?;
        let params = Self::get_params_iter(primary_key_arr);

        let num_rows = statement.execute(params_from_iter(params))?;
        if num_rows == 0 {
            Err(eyre!("Params returned no rows to delete."))
        } else {
            Ok(())
        }
    }

    /// Delete a specific entry using the current [Transaction].
    pub fn transaction_delete<T, U, const N: usize>(
        &mut self,
        primary_key_arr: [U; N],
        tx: Transaction,
    ) -> eyre::Result<()>
    where
        T: HasSqlStatements,
        U: IntoB64,
    {
        let num_rows: usize;

        {
            let mut statement = tx.prepare(T::sql_delete())?;
            let params = Self::get_params_iter(primary_key_arr);

            num_rows = statement.execute(params_from_iter(params))?;
        }

        if num_rows == 0 {
            Err(eyre!("Params returned no rows to delete."))
        } else {
            Ok(())
        }
    }

    /// Insert a specific entry into the matching table.
    pub fn insert_entry<T>(&self, entry: T) -> eyre::Result<()>
    where
        T: IntoDatabase + HasSqlStatements,
        T::FixedSizeStringArray: rusqlite::Params,
    {
        self.connection
            .execute(T::sql_insert(), entry.into_database())?;
        Ok(())
    }

    // Helper function to get SQLite params from an array.
    fn get_params_iter<U, const N: usize>(
        params_arr: [U; N],
    ) -> Map<IntoIter<U, N>, impl FnMut(U) -> String>
    where
        U: IntoB64,
    {
        params_arr.into_iter().map(|e| e.into_b64())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, remove_file, File},
        io::Write,
    };

    use pretty_assertions::assert_eq;

    use super::{
        super::super::{
            account::Account,
            credential::Credential,
            encryption::encrypted::{new_rand_key, Encrypted, TryFromEncrypted, TryIntoEncrypted},
            file_data::FileData,
        },
        *,
    };

    fn test_db_path(path_str: &str) -> Utf8PathBuf {
        Utf8PathBuf::from(path_str)
    }

    fn refresh_test_db(path_str: &str) -> Database {
        let _ = fs::remove_file(test_db_path(path_str));
        fs::File::create_new(test_db_path(path_str)).unwrap();
        Database::connect(test_db_path(path_str)).unwrap()
    }

    fn make_a_file(path: &Utf8Path, bytes: &[u8]) -> eyre::Result<()> {
        let mut f = File::create_new(path)?;
        f.write_all(bytes)?;
        Ok(())
    }

    fn delete_a_file(path: &Utf8Path) -> eyre::Result<()> {
        remove_file(path)?;
        Ok(())
    }

    #[test]
    fn account_to_from() {
        let db_path = "tests/account_to_from.db";
        let db = refresh_test_db(db_path);

        let username = "Mister Test";
        let password = "I'm the great Mister Test, I don't need a password!";
        let account = Account::new(username, password).unwrap();

        db.insert_entry(account.clone()).unwrap();
        let loaded_account: Account = db.select_entry([username]).unwrap().unwrap();

        assert_eq!(account, loaded_account);

        assert_eq!(loaded_account.username(), username);
    }

    #[test]
    fn credential_to_from() {
        let db_path = "tests/credential_to_from.db";
        let db = refresh_test_db(db_path);

        let owner_username = "mister_owner_123";
        let owner_password = "123";
        let different_owner_username = "not_mister_owner";
        let key = new_rand_key();
        let name = "maxgmr.ca login info";
        let username = "im_da_admin";
        let password = "blahblahblah";
        let notes = "dgruft很酷。";

        let cred =
            Credential::try_new(owner_username, key, name, username, password, notes).unwrap();

        // Trying to insert a credential without an existing, matching account should fail.
        let _ = db.insert_entry(cred.clone()).unwrap_err();

        let account = Account::new(owner_username, owner_password).unwrap();
        let other_account = Account::new(different_owner_username, owner_password).unwrap();

        db.insert_entry(other_account.clone()).unwrap();

        // There is still no account that matches. Should still fail.
        let _ = db.insert_entry(cred.clone()).unwrap_err();

        db.insert_entry(account.clone()).unwrap();

        db.insert_entry(cred.clone()).unwrap();
        let loaded_cred: Credential = db
            .select_entry([
                cred.owner_username().as_bytes(),
                cred.encrypted_name().cipherbytes(),
            ])
            .unwrap()
            .unwrap();

        assert_eq!(cred, loaded_cred);

        assert_eq!(loaded_cred.name::<String>(key).unwrap(), name);
        assert_eq!(loaded_cred.username::<String>(key).unwrap(), username);
        assert_eq!(loaded_cred.password::<String>(key).unwrap(), password);
        assert_eq!(loaded_cred.notes::<String>(key).unwrap(), notes);
    }

    #[test]
    fn file_data_to_from() {
        let db_path = "tests/file_data_to_from.db";
        let db = refresh_test_db(db_path);

        let path = Utf8PathBuf::from("src/backend/vault/database_traits.rs");
        let filename = String::from("database_traits.rs");
        let owner_username = String::from("i'm da owner");
        let owner_password = "open sesame!";
        let (encrypted_contents, key) = "test".try_encrypt_new_key().unwrap();
        let contents_nonce = encrypted_contents.nonce();

        let account = Account::new(&owner_username, owner_password).unwrap();
        db.insert_entry(account).unwrap();

        let file_data = FileData::new(
            path.clone(),
            filename.clone(),
            owner_username.clone(),
            contents_nonce,
        );

        db.insert_entry(file_data.clone()).unwrap();
        let loaded_file_data = db.select_entry([&path]).unwrap().unwrap();

        assert_eq!(file_data, loaded_file_data);

        assert_eq!(path, loaded_file_data.path());
        assert_eq!(filename, loaded_file_data.filename());
        assert_eq!(owner_username, loaded_file_data.owner_username());
        assert_eq!(contents_nonce, loaded_file_data.contents_nonce());

        let decrypted_contents = String::try_decrypt(
            &Encrypted::from_fields(
                encrypted_contents.cipherbytes().to_vec(),
                loaded_file_data.contents_nonce(),
            ),
            key,
        )
        .unwrap();
        assert_eq!(decrypted_contents, "test");
    }

    #[test]
    fn delete() {
        let db_path = "tests/delete.db";
        let db = refresh_test_db(db_path);

        let dir = Utf8PathBuf::from("tests/");

        let uname_1 = "mr_test";
        let pwd_1 = "i_love_testing_123";
        let acc_1 = Account::new(uname_1, pwd_1).unwrap();
        db.insert_entry(acc_1.clone()).unwrap();

        let uname_2 = "mr_awesome";
        let pwd_2 = "i am so so awesome!";
        let acc_2 = Account::new(uname_2, pwd_2).unwrap();
        db.insert_entry(acc_2.clone()).unwrap();

        let filename_1_1 = "f_1_1";
        let mut path_1_1 = dir.clone();
        path_1_1.push(filename_1_1);
        let (contents_1_1, key_1_1) = "test".try_encrypt_new_key().unwrap();
        let f_1_1 = FileData::new(
            &path_1_1,
            "f_1_1".to_string(),
            uname_1.to_string(),
            contents_1_1.nonce(),
        );
        db.insert_entry(f_1_1.clone()).unwrap();

        let filename_1_2 = "f_1_2";
        let mut path_1_2 = dir.clone();
        path_1_2.push(filename_1_2);
        let contents_1_2 = "test".try_encrypt_with_key(key_1_1).unwrap();
        let f_1_2 = FileData::new(
            &path_1_2,
            "f_1_2".to_string(),
            uname_1.to_string(),
            contents_1_2.nonce(),
        );
        db.insert_entry(f_1_2.clone()).unwrap();

        let filename_2_1 = "f_2_1";
        let mut path_2_1 = dir.clone();
        path_2_1.push(filename_2_1);
        let (contents_2_1, key_2_1) = "test".try_encrypt_new_key().unwrap();
        let f_2_1 = FileData::new(
            &path_2_1,
            "f_2_1".to_string(),
            uname_2.to_string(),
            contents_2_1.nonce(),
        );
        db.insert_entry(f_2_1.clone()).unwrap();

        let cred_1 = Credential::try_new(uname_1, key_1_1, "cred_1", "u1", "p1", "").unwrap();
        db.insert_entry(cred_1.clone()).unwrap();
        let cred_2 = Credential::try_new(uname_2, key_2_1, "cred_2", "u2", "p2", "").unwrap();
        db.insert_entry(cred_2.clone()).unwrap();

        assert!(db
            .select_entry::<Account, &str, 1>([uname_1])
            .unwrap()
            .is_some());
        assert!(db
            .select_entry::<FileData, &Utf8Path, 1>([&path_1_1])
            .unwrap()
            .is_some());
        assert!(db
            .select_entry::<Credential, &[u8], 2>([
                cred_1.owner_username().as_bytes(),
                cred_1.encrypted_name().cipherbytes()
            ])
            .unwrap()
            .is_some());

        db.delete_entry::<Account, &str, 1>([uname_1]).unwrap();
        assert!(db
            .select_entry::<Account, &str, 1>([uname_1])
            .unwrap()
            .is_none());
        assert!(db
            .select_entry::<Credential, &[u8], 2>([
                cred_1.owner_username().as_bytes(),
                cred_1.encrypted_name().cipherbytes()
            ])
            .unwrap()
            .is_none());
        assert!(db
            .select_entry::<FileData, &Utf8Path, 1>([&path_1_1])
            .unwrap()
            .is_none());
        assert!(db
            .select_entry::<FileData, &Utf8Path, 1>([&path_1_2])
            .unwrap()
            .is_none());
        assert!(db
            .select_entry::<Account, &str, 1>([uname_2])
            .unwrap()
            .is_some());
        assert!(db
            .select_entry::<FileData, &Utf8Path, 1>([&path_2_1])
            .unwrap()
            .is_some());
        assert!(db
            .select_entry::<Credential, &[u8], 2>([
                cred_2.owner_username().as_bytes(),
                cred_2.encrypted_name().cipherbytes()
            ])
            .unwrap()
            .is_some());

        db.delete_entry::<Credential, &[u8], 2>([
            cred_2.owner_username().as_bytes(),
            cred_2.encrypted_name().cipherbytes(),
        ])
        .unwrap();
        assert!(db
            .select_entry::<Account, &str, 1>([uname_2])
            .unwrap()
            .is_some());
        assert!(db
            .select_entry::<FileData, &Utf8Path, 1>([&path_2_1])
            .unwrap()
            .is_some());
        assert!(db
            .select_entry::<Credential, &[u8], 2>([
                cred_2.owner_username().as_bytes(),
                cred_2.encrypted_name().cipherbytes()
            ])
            .unwrap()
            .is_none());

        db.delete_entry::<FileData, &Utf8Path, 1>([&path_2_1])
            .unwrap();
        assert!(db
            .select_entry::<Account, &str, 1>([uname_2])
            .unwrap()
            .is_some());
        assert!(db
            .select_entry::<FileData, &Utf8Path, 1>([&path_2_1])
            .unwrap()
            .is_none());
        assert!(db
            .select_entry::<Credential, &[u8], 2>([
                cred_2.owner_username().as_bytes(),
                cred_2.encrypted_name().cipherbytes()
            ])
            .unwrap()
            .is_none());
    }

    #[test]
    fn rollback_delete_fail() {
        let file_path = Utf8PathBuf::from("tests/delete-rollback-test.txt");
        let _ = delete_a_file(&file_path);

        let db_path = "tests/rollback_delete_fail.db";
        let mut db = refresh_test_db(db_path);

        let username = "abc";
        let password = "123";
        let account = Account::new(username, password).unwrap();

        db.insert_entry(account).unwrap();
        make_a_file(&file_path, b"blah blah blah").unwrap();
        fs::metadata(&file_path).unwrap();

        // match db.transaction_delete::<Credential, &str, 1>([
        //     "wrong primary key field count! please preserve my file!",
        // ]) {
        //     Ok(_) => {}
        //     Err(_) => db.rollback_transaction().unwrap(),
        // };
        // match delete_a_file(&file_path) {
        //     Ok(_) => db.commit_transaction().unwrap(),
        //     Err(_) => db.rollback_transaction().unwrap(),
        // };

        fs::metadata(&file_path).unwrap();

        // match db.transaction_delete::<Account, &str, 1>([
        //     "misspelled username! i hope my file doesn't actually get deleted!",
        // ]) {
        //     Ok(_) => {}
        //     Err(_) => db.rollback_transaction().unwrap(),
        // };
        // match delete_a_file(&file_path) {
        //     Ok(_) => db.commit_transaction().unwrap(),
        //     Err(_) => db.rollback_transaction().unwrap(),
        // };

        fs::metadata(&file_path).unwrap();

        // match db.transaction_delete::<Account, &str, 1>(["abc"]) {
        //     Ok(_) => {}
        //     Err(_) => db.rollback_transaction().unwrap(),
        // };
        // match delete_a_file(&file_path) {
        //     Ok(_) => db.commit_transaction().unwrap(),
        //     Err(_) => db.rollback_transaction().unwrap(),
        // };

        fs::metadata(&file_path).unwrap_err();
    }

    #[test]
    fn rollback_insert_fail() {
        let file_path = Utf8PathBuf::from("tests/insert-rollback-test.txt");
        let _ = delete_a_file(&file_path);

        let db_path = "tests/rollback_insert_fail.db";
        let mut db = refresh_test_db(db_path);

        let username = "abc";
        let password = "123";
        let account = Account::new(username, password).unwrap();

        // db.transaction_insert(account, make_a_file(&file_path, b"blah blah blah"))
        //     .unwrap();
        // fs::metadata(&file_path).unwrap();

        // let _ = db
        //     .transaction_delete::<Credential, &str, 1>(
        //         ["wrong primary key field count! please preserve my file!"],
        //         delete_a_file(&file_path),
        //     )
        //     .unwrap_err();

        fs::metadata(&file_path).unwrap();

        // let _ = db
        //     .transaction_delete::<Account, &str, 1>(
        //         ["misspelled username! i hope my file doesn't actually get deleted!"],
        //         delete_a_file(&file_path),
        //     )
        //     .unwrap_err();

        fs::metadata(&file_path).unwrap();

        // db.transaction_delete::<Account, &str, 1>(["abc"], delete_a_file(&file_path))
        //     .unwrap();

        fs::metadata(&file_path).unwrap_err();
    }
}
