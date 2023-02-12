// Licensed to the Apache Software Foundation (ASF) under one
// or more contributor license agreements.  See the NOTICE file
// distributed with this work for additional information
// regarding copyright ownership.  The ASF licenses this file
// to you under the Apache License, Version 2.0 (the
// "License"); you may not use this file except in compliance
// with the License.  You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing,
// software distributed under the License is distributed on an
// "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.  See the License for the
// specific language governing permissions and limitations
// under the License.

use std::{
    cell::RefCell,
    rc::Rc,
    sync::{Arc, Mutex},
};

use arrow::{
    datatypes::{DataType, Field, Schema},
    error::ArrowError,
    record_batch::{RecordBatch, RecordBatchReader},
};
use arrow_adbc::{
    adbc_init_func,
    driver_manager::{AdbcDatabaseBuilder, AdbcDriver, AdbcDriverInitFunc},
    error::{AdbcError, AdbcStatusCode},
    ffi::AdbcObjectDepth,
    implement::{AdbcConnectionImpl, AdbcDatabaseImpl, AdbcStatementImpl},
    interface::{
        ConnectionApi, DatabaseApi, PartitionedStatementResult, StatementApi, StatementResult,
    },
    ADBC_VERSION_1_0_0,
};
use itertools::iproduct;

enum TestError {
    General(String),
}

impl TestError {
    pub fn new(msg: impl Into<String>) -> Self {
        Self::General(msg.into())
    }
}

impl AdbcError for TestError {
    fn message(&self) -> &str {
        match self {
            Self::General(msg) => msg,
        }
    }

    fn status_code(&self) -> AdbcStatusCode {
        AdbcStatusCode::Internal
    }
}

type Result<T> = std::result::Result<T, TestError>;

type ConnectionGetObjects = dyn Fn(
        AdbcObjectDepth,
        Option<&str>,
        Option<&str>,
        Option<&str>,
        Option<&[&str]>,
        Option<&str>,
    ) -> Result<Box<dyn RecordBatchReader>>
    + Send
    + Sync;

/// Contains closures for every ADBC method.
///
/// These methods are all set to return a "Not implemented" error by default.
/// Tests will dynamically set these functions to create implementations to test.
#[allow(clippy::type_complexity)]
struct PatchableDriver {
    database_set_option: Box<dyn Fn(&str, &str) -> Result<()> + Send + Sync>,
    connection_set_option: Box<dyn Fn(&str, &str) -> Result<()> + Send + Sync>,
    connection_get_info: Box<dyn Fn(&[u32]) -> Result<Box<dyn RecordBatchReader>> + Send + Sync>,
    connection_get_objects: Box<ConnectionGetObjects>,
    connection_get_table_schema:
        Box<dyn Fn(Option<&str>, Option<&str>, &str) -> Result<Schema> + Send + Sync>,
    connection_get_table_types: Box<dyn Fn() -> Result<Vec<String>> + Send + Sync>,
    connection_read_partition:
        Box<dyn Fn(&[u8]) -> Result<Box<dyn RecordBatchReader>> + Send + Sync>,
    connection_rollback: Box<dyn Fn() -> Result<()> + Send + Sync>,
    connection_commit: Box<dyn Fn() -> Result<()> + Send + Sync>,
}

macro_rules! patch_stub {
    ($($arg:tt),*) => {
        Box::new(|$($arg),*| Err(TestError::General("Not implemented".to_string())))
    };
}

impl Default for PatchableDriver {
    fn default() -> Self {
        Self {
            database_set_option: patch_stub!(_, _),
            connection_set_option: patch_stub!(_, _),
            connection_get_info: patch_stub!(_),
            connection_get_objects: patch_stub!(_, _, _, _, _, _),
            connection_get_table_schema: patch_stub!(_, _, _),
            connection_get_table_types: patch_stub!(),
            connection_read_partition: patch_stub!(_),
            connection_rollback: patch_stub!(),
            connection_commit: patch_stub!(),
        }
    }
}

/// A database whose implementation is backed by a [PatchableDriver].
///
/// When created, a reference to the inner [PatchableDriver] is copied into the
/// thread local [PATCH_HANDOFF]. The caller should retrieve that handle for use
/// in the tests.
struct TestDatabase {
    driver: Arc<Mutex<PatchableDriver>>,
}

thread_local! {
    static PATCH_HANDOFF: RefCell<Option<Arc<Mutex<PatchableDriver>>>> = RefCell::new(None);
}

impl Default for TestDatabase {
    fn default() -> Self {
        let driver = Arc::new(Mutex::new(PatchableDriver::default()));

        // Send a copy to global state
        PATCH_HANDOFF.with(|handoff| handoff.borrow_mut().replace(driver.clone()));

        Self { driver }
    }
}

impl AdbcDatabaseImpl for TestDatabase {
    fn init(&self) -> Result<()> {
        Ok(())
    }
}

impl DatabaseApi for TestDatabase {
    type Error = TestError;

    fn set_option(&self, key: &str, value: &str) -> Result<()> {
        (self.driver.lock().unwrap().database_set_option)(key, value)
    }
}

struct TestConnection {
    database: RefCell<Option<Arc<TestDatabase>>>,
}

impl TestConnection {
    fn get_driver_impl(&self) -> Result<Arc<Mutex<PatchableDriver>>> {
        if let Some(database) = self.database.borrow_mut().as_mut() {
            Ok(database.driver.clone())
        } else {
            Err(TestError::new("Connection not initialized"))
        }
    }
}

impl Default for TestConnection {
    fn default() -> Self {
        Self {
            database: RefCell::new(None),
        }
    }
}

impl AdbcConnectionImpl for TestConnection {
    type DatabaseType = TestDatabase;

    fn init(&self, database: Arc<Self::DatabaseType>) -> Result<()> {
        if self.database.borrow().is_none() {
            self.database.replace(Some(database));
            Ok(())
        } else {
            Err(TestError::General(
                "Already called init on the connection.".to_string(),
            ))
        }
    }
}

macro_rules! conn_method {
    ($self:expr, $func_name:ident, $($arg:expr),*) => {
        ($self.get_driver_impl()?.lock().unwrap().$func_name)($($arg),*)
    };
    ($self:expr, $func_name:ident) => {
        ($self.get_driver_impl()?.lock().unwrap().$func_name)()
    };
}

impl ConnectionApi for TestConnection {
    type Error = TestError;
    fn set_option(&self, key: &str, value: &str) -> Result<()> {
        conn_method!(self, connection_set_option, key, value)
    }

    fn get_info(&self, info_codes: &[u32]) -> Result<Box<dyn RecordBatchReader>> {
        conn_method!(self, connection_get_info, info_codes)
    }

    fn get_objects(
        &self,
        depth: AdbcObjectDepth,
        catalog: Option<&str>,
        db_schema: Option<&str>,
        table_name: Option<&str>,
        table_type: Option<&[&str]>,
        column_name: Option<&str>,
    ) -> Result<Box<dyn RecordBatchReader>> {
        conn_method!(
            self,
            connection_get_objects,
            depth,
            catalog,
            db_schema,
            table_name,
            table_type,
            column_name
        )
    }

    fn get_table_schema(
        &self,
        catalog: Option<&str>,
        db_schema: Option<&str>,
        table_name: &str,
    ) -> Result<Schema> {
        conn_method!(
            self,
            connection_get_table_schema,
            catalog,
            db_schema,
            table_name
        )
    }

    fn get_table_types(&self) -> Result<Vec<String>> {
        conn_method!(self, connection_get_table_types)
    }

    fn read_partition(&self, partition: &[u8]) -> Result<Box<dyn RecordBatchReader>> {
        conn_method!(self, connection_read_partition, partition)
    }

    fn rollback(&self) -> Result<()> {
        conn_method!(self, connection_rollback)
    }

    fn commit(&self) -> Result<()> {
        conn_method!(self, connection_commit)
    }
}

struct TestStatement {
    _connection: Rc<TestConnection>,
}

impl AdbcStatementImpl for TestStatement {
    type ConnectionType = TestConnection;

    fn new_from_connection(connection: Rc<Self::ConnectionType>) -> Self {
        Self {
            _connection: connection,
        }
    }
}

impl StatementApi for TestStatement {
    type Error = TestError;

    fn set_option(&mut self, key: &str, value: &str) -> Result<()> {
        Err(TestError::General(format!(
            "Not implemented: setting option with key '{key}' and value '{value}'."
        )))
    }

    fn set_sql_query(&mut self, query: &str) -> Result<()> {
        Err(TestError::General(format!(
            "Not implemented: setting query '{query}'."
        )))
    }

    fn set_substrait_plan(&mut self, plan: &[u8]) -> Result<()> {
        Err(TestError::General(format!(
            "Not implemented: setting plan '{plan:?}'."
        )))
    }

    fn prepare(&mut self) -> Result<()> {
        Err(TestError::General(
            "Not implemented: preparing statement.".to_string(),
        ))
    }

    fn get_param_schema(&mut self) -> Result<Schema> {
        Err(TestError::General(
            "Not implemented: get parameter schema.".to_string(),
        ))
    }

    fn bind_data(&mut self, arr: RecordBatch) -> Result<()> {
        Err(TestError::General(format!(
            "Not implemented: binding data {arr:?}."
        )))
    }

    fn bind_stream(&mut self, stream: Box<dyn RecordBatchReader>) -> Result<()> {
        let batches: Vec<RecordBatch> = stream
            .collect::<std::result::Result<_, ArrowError>>()
            .map_err(|_| TestError::General("Error collecting stream.".to_string()))?;

        Err(TestError::General(format!(
            "Not implemented: binding stream {batches:?}."
        )))
    }

    fn execute(&mut self) -> Result<StatementResult> {
        Err(TestError::General("Not implemented: execute".to_string()))
    }

    fn execute_update(&mut self) -> Result<i64> {
        Err(TestError::General("Not implemented: execute".to_string()))
    }

    fn execute_partitioned(&mut self) -> Result<PartitionedStatementResult> {
        Err(TestError::General(
            "Not implemented: execute partitioned".to_string(),
        ))
    }
}

adbc_init_func!(TestDriverInit, TestStatement);

// TODO: test unsafe parts of API for basic handling of null or even unaligned pointers.

fn get_driver() -> AdbcDriver {
    AdbcDriver::load_from_init(&(TestDriverInit as AdbcDriverInitFunc), ADBC_VERSION_1_0_0).unwrap()
}

fn get_database_builder() -> (AdbcDatabaseBuilder, Arc<Mutex<PatchableDriver>>) {
    let driver = get_driver();
    let builder = driver.new_database().unwrap();
    let mock_driver = PATCH_HANDOFF
        .with(|handoff| handoff.borrow_mut().take())
        .expect("Failed to get reference to patchable driver.");
    (builder, mock_driver)
}

macro_rules! set_driver_method {
    ($driver:expr, $func_name:ident, $closure:expr) => {
        $driver.lock().unwrap().$func_name = Box::new($closure);
    };
}

#[test]
fn test_database_set_option() {
    let (builder, mock_driver) = get_database_builder();

    set_driver_method!(
        mock_driver,
        database_set_option,
        |key: &str, value: &str| {
            assert_eq!(key, "test_key");
            assert_eq!(value, "test value 😬");
            Ok(())
        }
    );

    let builder = builder.set_option("test_key", "test value 😬").unwrap();
    let database = builder.init().unwrap();
    database.set_option("test_key", "test value 😬").unwrap();

    set_driver_method!(mock_driver, database_set_option, |_: &str, _: &str| {
        Err(TestError::new("hello world"))
    });

    let res = database.set_option("key", "value");
    assert!(res.is_err());
    assert_eq!(res.unwrap_err().message, "hello world");
}

#[test]
fn test_connection_set_option() {
    let (builder, mock_driver) = get_database_builder();
    let conn_builder = builder.init().unwrap().new_connection().unwrap();

    set_driver_method!(
        mock_driver,
        connection_set_option,
        |key: &str, value: &str| {
            assert_eq!(key, "test_key");
            assert_eq!(value, "test value 😬");
            Ok(())
        }
    );
    let conn = conn_builder.init().unwrap();
    conn.set_option("test_key", "test value 😬").unwrap();

    set_driver_method!(mock_driver, connection_set_option, |_: &str, _: &str| {
        Err(TestError::new("hello world"))
    });

    let res = conn.set_option("key", "value");
    assert!(res.is_err());
    assert_eq!(res.unwrap_err().message, "hello world");
}

#[test]
fn test_connection_get_info() {
    todo!()
}

#[test]
fn test_connection_get_objects() {
    todo!()
}

#[test]
fn test_connection_get_table_schema() {
    let (builder, mock_driver) = get_database_builder();
    let conn = builder
        .init()
        .unwrap()
        .new_connection()
        .unwrap()
        .init()
        .unwrap();

    // let catalogs = vec![None, Some("my_catalog"), Some("")];
    let catalogs = vec![Some("my_catalog")];
    // let db_schemas = vec![None, Some("my_schema"), Some("")];
    let db_schemas = vec![Some("my_schema")];
    let table_names = vec!["my_table", ""];

    let test_schema = Schema::new(vec![Field::new("x", DataType::Int64, true)]);

    for (catalog, db_schema, table_name) in iproduct!(catalogs, db_schemas, table_names) {
        let expected_catalog = catalog;
        let expected_db_schema = db_schema;
        let expected_table = table_name;
        let out_schema = test_schema.clone();

        set_driver_method!(
            mock_driver,
            connection_get_table_schema,
            move |catalog, db_schema, table_name| {
                assert_eq!(catalog, expected_catalog);
                assert_eq!(db_schema, expected_db_schema);
                assert_eq!(table_name, expected_table);
                Ok(out_schema.clone())
            }
        );
        let table_schema = conn
            .get_table_schema(catalog, db_schema, table_name)
            .unwrap();
        assert_eq!(table_schema, test_schema);
    }

    set_driver_method!(mock_driver, connection_get_table_schema, move |_, _, _| {
        Err(TestError::new("hello world"))
    });
    let res = conn.get_table_schema(None, None, "");
    assert!(res.is_err());
    assert_eq!(res.unwrap_err().message, "hello world");
}

#[test]
fn test_connection_get_table_types() {
    let (builder, mock_driver) = get_database_builder();
    let conn = builder
        .init()
        .unwrap()
        .new_connection()
        .unwrap()
        .init()
        .unwrap();

    let cases = vec![vec![], vec!["one"], vec!["hello", "你好"]];

    for expected in cases {
        let to_return = expected.clone();
        set_driver_method!(mock_driver, connection_get_table_types, move || {
            Ok(to_return.iter().map(|s| s.to_string()).collect())
        });

        let table_types = conn.get_table_types().unwrap();
        assert_eq!(table_types, expected);
    }

    set_driver_method!(mock_driver, connection_get_table_types, move || {
        Err(TestError::new("hello world"))
    });
    let res = conn.get_table_types();
    assert!(res.is_err());
    assert_eq!(res.unwrap_err().message, "hello world");
}