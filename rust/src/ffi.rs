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

//! ADBC FFI structs, as defined in [adbc.h](https://github.com/apache/arrow-adbc/blob/main/adbc.h).
use std::ffi::{c_char, c_void, CStr};
use std::ptr::{null, null_mut};

use crate::error::{AdbcStatusCode, FFI_AdbcError};
use arrow::ffi::{FFI_ArrowArray, FFI_ArrowSchema};
use arrow::ffi_stream::FFI_ArrowArrayStream;

/// An instance of a database.
///
/// Must be kept alive as long as any connections exist.
#[repr(C)]
#[derive(Debug, Clone)]
pub struct FFI_AdbcDatabase {
    /// Opaque implementation-defined state.
    /// This field is NULLPTR iff the connection is unintialized/freed.
    pub private_data: *mut c_void,
    /// The associated driver (used by the driver manager to help
    /// track state).
    pub private_driver: *const FFI_AdbcDriver,
}

impl FFI_AdbcDatabase {
    pub fn empty() -> Self {
        Self {
            private_data: null_mut(),
            private_driver: null_mut(),
        }
    }
}

impl Drop for FFI_AdbcDatabase {
    fn drop(&mut self) {
        if let Some(private_driver) = unsafe { self.private_driver.as_ref() } {
            if let Some(release) = private_driver.database_release {
                let mut error = FFI_AdbcError::empty();
                let status = unsafe { release(self, &mut error) };
                if status != AdbcStatusCode::Ok {
                    panic!("Failed to cleanup database: {}", unsafe {
                        CStr::from_ptr(error.message).to_string_lossy()
                    });
                }
            }
        }
    }
}

/// An active database connection.
///
/// Provides methods for query execution, managing prepared
/// statements, using transactions, and so on.
///
/// Connections are not required to be thread-safe, but they can be
/// used from multiple threads so long as clients take care to
/// serialize accesses to a connection. Because of this, they do not implement
/// [core::marker::Send] + [core::marker::Sync] on their own. Instead wrap them
/// in the appropriate types to manage access safely and implement those marker
/// traits on the wrapper.
#[repr(C)]
#[derive(Debug, Clone)]
pub struct FFI_AdbcConnection {
    /// Opaque implementation-defined state.
    /// This field is NULLPTR iff the connection is unintialized/freed.
    pub private_data: *mut c_void,
    /// The associated driver (used by the driver manager to help
    ///   track state).
    pub private_driver: *mut FFI_AdbcDriver,
}

impl FFI_AdbcConnection {
    pub fn empty() -> Self {
        Self {
            private_data: null_mut(),
            private_driver: null_mut(),
        }
    }
}

impl Drop for FFI_AdbcConnection {
    fn drop(&mut self) {
        if let Some(private_driver) = unsafe { self.private_driver.as_ref() } {
            if let Some(release) = private_driver.connection_release.as_ref() {
                let mut error = FFI_AdbcError::empty();
                let status = unsafe { release(self, &mut error) };
                if status != AdbcStatusCode::Ok {
                    panic!("Failed to cleanup connection: {}", unsafe {
                        CStr::from_ptr(error.message).to_string_lossy()
                    });
                }
            }
        }
    }
}

/// A container for all state needed to execute a database
/// query, such as the query itself, parameters for prepared
/// statements, driver parameters, etc.
///
/// Statements may represent queries or prepared statements.
///
/// Statements may be used multiple times and can be reconfigured
/// (e.g. they can be reused to execute multiple different queries).
/// However, executing a statement (and changing certain other state)
/// will invalidate result sets obtained prior to that execution.
///
/// Multiple statements may be created from a single connection.
/// However, the driver may block or error if they are used
/// concurrently (whether from a single thread or multiple threads).
///
/// Statements are not required to be thread-safe, but they can be
/// used from multiple threads so long as clients take care to
/// serialize accesses to a statement.
#[repr(C)]
#[derive(Debug, Clone)]
pub struct FFI_AdbcStatement {
    /// Opaque implementation-defined state.
    /// This field is NULLPTR iff the connection is unintialized/freed.
    pub private_data: *mut c_void,
    /// The associated driver (used by the driver manager to help
    /// track state).
    pub private_driver: *mut FFI_AdbcDriver,
}

impl FFI_AdbcStatement {
    pub fn empty() -> Self {
        Self {
            private_data: null_mut(),
            private_driver: null_mut(),
        }
    }
}

impl Drop for FFI_AdbcStatement {
    fn drop(&mut self) {
        if let Some(private_driver) = unsafe { self.private_driver.as_ref() } {
            if let Some(release) = private_driver.statement_release {
                let mut error = FFI_AdbcError::empty();
                let status = unsafe { release(self, &mut error) };
                if status != AdbcStatusCode::Ok {
                    panic!("Failed to cleanup statement: {}", unsafe {
                        CStr::from_ptr(error.message).to_string_lossy()
                    });
                }
            }
        }
    }
}

/// The partitions of a distributed/partitioned result set.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct FFI_AdbcPartitions {
    /// The number of partitions.
    pub num_partitions: usize,
    /// The partitions of the result set, where each entry (up to
    /// num_partitions entries) is an opaque identifier that can be
    /// passed to FFI_AdbcConnectionReadPartition.
    pub partitions: *mut *const u8,
    /// The length of each corresponding entry in partitions.
    pub partition_lengths: *const usize,
    /// Opaque implementation-defined state.
    /// This field is NULLPTR iff the connection is unintialized/freed.
    pub private_data: *mut c_void,
    /// Release the contained partitions.
    ///
    /// Unlike other structures, this is an embedded callback to make it
    /// easier for the driver manager and driver to cooperate.
    pub release: ::std::option::Option<unsafe extern "C" fn(partitions: *mut FFI_AdbcPartitions)>,
}

impl FFI_AdbcPartitions {
    pub fn empty() -> Self {
        Self {
            num_partitions: 0,
            partitions: null_mut(),
            partition_lengths: null(),
            private_data: null_mut(),
            release: None,
        }
    }
}

impl From<Vec<Vec<u8>>> for FFI_AdbcPartitions {
    fn from(mut value: Vec<Vec<u8>>) -> Self {
        // Make sure capacity and length are the same, so it's easier to reconstruct them.
        value.shrink_to_fit();

        let num_partitions = value.len();
        let mut lengths: Vec<usize> = value.iter().map(|v| v.len()).collect();
        let partition_lengths = lengths.as_mut_ptr();
        std::mem::forget(lengths);

        let mut partitions_vec: Vec<*const u8> = value
            .into_iter()
            .map(|mut p| {
                p.shrink_to_fit();
                let ptr = p.as_ptr();
                std::mem::forget(p);
                ptr
            })
            .collect();
        partitions_vec.shrink_to_fit();
        let partitions = partitions_vec.as_mut_ptr();
        std::mem::forget(partitions_vec);

        Self {
            num_partitions,
            partitions,
            partition_lengths,
            private_data: 42 as *mut c_void, // Arbitrary non-null pointer
            release: Some(drop_adbc_partitions),
        }
    }
}

unsafe extern "C" fn drop_adbc_partitions(partitions: *mut FFI_AdbcPartitions) {
    if let Some(partitions) = partitions.as_mut() {
        // This must reconstruct every Vec that we called mem::forget on when
        // constructing the FFI struct.
        let partition_lengths: Vec<usize> = Vec::from_raw_parts(
            partitions.partition_lengths as *mut usize,
            partitions.num_partitions,
            partitions.num_partitions,
        );

        let partitions_vec = Vec::from_raw_parts(
            partitions.partitions,
            partitions.num_partitions,
            partitions.num_partitions,
        );

        let _each_partition: Vec<Vec<u8>> = partitions_vec
            .into_iter()
            .zip(partition_lengths)
            .map(|(ptr, size)| Vec::from_raw_parts(ptr as *mut u8, size, size))
            .collect();

        partitions.partitions = null_mut();
        partitions.partition_lengths = null_mut();
        partitions.private_data = null_mut();
        partitions.release = None;
    }
}

/// An instance of an initialized database driver.
///
/// This provides a common interface for vendor-specific driver
/// initialization routines. Drivers should populate this struct, and
/// applications can call ADBC functions through this struct, without
/// worrying about multiple definitions of the same symbol.
#[repr(C)]
#[derive(Debug, Clone)]
pub struct FFI_AdbcDriver {
    /// Opaque driver-defined state.
    /// This field is NULL if the driver is unintialized/freed (but
    /// it need not have a value even if the driver is initialized).
    pub private_data: *mut c_void,
    /// Opaque driver manager-defined state.
    /// This field is NULL if the driver is unintialized/freed (but
    /// it need not have a value even if the driver is initialized).
    pub private_manager: *mut c_void,
    ///  Release the driver and perform any cleanup.
    ///
    /// This is an embedded callback to make it easier for the driver
    /// manager and driver to cooperate.
    pub release: ::std::option::Option<
        unsafe extern "C" fn(
            driver: *mut FFI_AdbcDriver,
            error: *mut FFI_AdbcError,
        ) -> AdbcStatusCode,
    >,
    pub database_init: ::std::option::Option<
        unsafe extern "C" fn(
            arg1: *mut FFI_AdbcDatabase,
            arg2: *mut FFI_AdbcError,
        ) -> AdbcStatusCode,
    >,
    pub database_new: ::std::option::Option<
        unsafe extern "C" fn(
            arg1: *mut FFI_AdbcDatabase,
            arg2: *mut FFI_AdbcError,
        ) -> AdbcStatusCode,
    >,
    pub database_set_option: ::std::option::Option<
        unsafe extern "C" fn(
            arg1: *mut FFI_AdbcDatabase,
            arg2: *const c_char,
            arg3: *const c_char,
            arg4: *mut FFI_AdbcError,
        ) -> AdbcStatusCode,
    >,
    pub database_release: ::std::option::Option<
        unsafe extern "C" fn(
            arg1: *mut FFI_AdbcDatabase,
            arg2: *mut FFI_AdbcError,
        ) -> AdbcStatusCode,
    >,
    pub connection_commit: ::std::option::Option<
        unsafe extern "C" fn(
            arg1: *mut FFI_AdbcConnection,
            arg2: *mut FFI_AdbcError,
        ) -> AdbcStatusCode,
    >,
    pub connection_get_info: ::std::option::Option<
        unsafe extern "C" fn(
            arg1: *mut FFI_AdbcConnection,
            arg2: *const u32,
            arg3: usize,
            arg4: *mut FFI_ArrowArrayStream,
            arg5: *mut FFI_AdbcError,
        ) -> AdbcStatusCode,
    >,
    pub connection_get_objects: ::std::option::Option<
        unsafe extern "C" fn(
            arg1: *mut FFI_AdbcConnection,
            arg2: AdbcObjectDepth,
            arg3: *const c_char,
            arg4: *const c_char,
            arg5: *const c_char,
            arg6: *const *const c_char,
            arg7: *const c_char,
            arg8: *mut FFI_ArrowArrayStream,
            arg9: *mut FFI_AdbcError,
        ) -> AdbcStatusCode,
    >,
    pub connection_get_table_schema: ::std::option::Option<
        unsafe extern "C" fn(
            arg1: *mut FFI_AdbcConnection,
            arg2: *const c_char,
            arg3: *const c_char,
            arg4: *const c_char,
            arg5: *mut FFI_ArrowSchema,
            arg6: *mut FFI_AdbcError,
        ) -> AdbcStatusCode,
    >,
    pub connection_get_table_types: ::std::option::Option<
        unsafe extern "C" fn(
            arg1: *mut FFI_AdbcConnection,
            arg2: *mut FFI_ArrowArrayStream,
            arg3: *mut FFI_AdbcError,
        ) -> AdbcStatusCode,
    >,
    pub connection_init: ::std::option::Option<
        unsafe extern "C" fn(
            arg1: *mut FFI_AdbcConnection,
            arg2: *mut FFI_AdbcDatabase,
            arg3: *mut FFI_AdbcError,
        ) -> AdbcStatusCode,
    >,
    pub connection_new: ::std::option::Option<
        unsafe extern "C" fn(
            arg1: *mut FFI_AdbcConnection,
            arg2: *mut FFI_AdbcError,
        ) -> AdbcStatusCode,
    >,
    pub connection_set_option: ::std::option::Option<
        unsafe extern "C" fn(
            arg1: *mut FFI_AdbcConnection,
            arg2: *const c_char,
            arg3: *const c_char,
            arg4: *mut FFI_AdbcError,
        ) -> AdbcStatusCode,
    >,
    pub connection_read_partition: ::std::option::Option<
        unsafe extern "C" fn(
            arg1: *mut FFI_AdbcConnection,
            arg2: *const u8,
            arg3: usize,
            arg4: *mut FFI_ArrowArrayStream,
            arg5: *mut FFI_AdbcError,
        ) -> AdbcStatusCode,
    >,
    pub connection_release: ::std::option::Option<
        unsafe extern "C" fn(
            arg1: *mut FFI_AdbcConnection,
            arg2: *mut FFI_AdbcError,
        ) -> AdbcStatusCode,
    >,
    pub connection_rollback: ::std::option::Option<
        unsafe extern "C" fn(
            arg1: *mut FFI_AdbcConnection,
            arg2: *mut FFI_AdbcError,
        ) -> AdbcStatusCode,
    >,
    pub statement_bind: ::std::option::Option<
        unsafe extern "C" fn(
            arg1: *mut FFI_AdbcStatement,
            arg2: *mut FFI_ArrowArray,
            arg3: *mut FFI_ArrowSchema,
            arg4: *mut FFI_AdbcError,
        ) -> AdbcStatusCode,
    >,
    pub statement_bind_stream: ::std::option::Option<
        unsafe extern "C" fn(
            arg1: *mut FFI_AdbcStatement,
            arg2: *mut FFI_ArrowArrayStream,
            arg3: *mut FFI_AdbcError,
        ) -> AdbcStatusCode,
    >,
    pub statement_execute_query: ::std::option::Option<
        unsafe extern "C" fn(
            arg1: *mut FFI_AdbcStatement,
            arg2: *mut FFI_ArrowArrayStream,
            arg3: *mut i64,
            arg4: *mut FFI_AdbcError,
        ) -> AdbcStatusCode,
    >,
    pub statement_execute_partitions: ::std::option::Option<
        unsafe extern "C" fn(
            arg1: *mut FFI_AdbcStatement,
            arg2: *mut FFI_ArrowSchema,
            arg3: *mut FFI_AdbcPartitions,
            arg4: *mut i64,
            arg5: *mut FFI_AdbcError,
        ) -> AdbcStatusCode,
    >,
    pub statement_get_parameter_schema: ::std::option::Option<
        unsafe extern "C" fn(
            arg1: *mut FFI_AdbcStatement,
            arg2: *mut FFI_ArrowSchema,
            arg3: *mut FFI_AdbcError,
        ) -> AdbcStatusCode,
    >,
    pub statement_new: ::std::option::Option<
        unsafe extern "C" fn(
            arg1: *mut FFI_AdbcConnection,
            arg2: *mut FFI_AdbcStatement,
            arg3: *mut FFI_AdbcError,
        ) -> AdbcStatusCode,
    >,
    pub statement_prepare: ::std::option::Option<
        unsafe extern "C" fn(
            arg1: *mut FFI_AdbcStatement,
            arg2: *mut FFI_AdbcError,
        ) -> AdbcStatusCode,
    >,
    pub statement_release: ::std::option::Option<
        unsafe extern "C" fn(
            arg1: *mut FFI_AdbcStatement,
            arg2: *mut FFI_AdbcError,
        ) -> AdbcStatusCode,
    >,
    pub statement_set_option: ::std::option::Option<
        unsafe extern "C" fn(
            arg1: *mut FFI_AdbcStatement,
            arg2: *const c_char,
            arg3: *const c_char,
            arg4: *mut FFI_AdbcError,
        ) -> AdbcStatusCode,
    >,
    pub statement_set_sql_query: ::std::option::Option<
        unsafe extern "C" fn(
            arg1: *mut FFI_AdbcStatement,
            arg2: *const c_char,
            arg3: *mut FFI_AdbcError,
        ) -> AdbcStatusCode,
    >,
    pub statement_set_substrait_plan: ::std::option::Option<
        unsafe extern "C" fn(
            arg1: *mut FFI_AdbcStatement,
            arg2: *const u8,
            arg3: usize,
            arg4: *mut FFI_AdbcError,
        ) -> AdbcStatusCode,
    >,
}

macro_rules! empty_driver {
    ($( $func_name:ident ),+) => {
        Self {
            private_data: null_mut(),
            private_manager: null_mut(),
            release: None,
            $(
                $func_name: Some(driver_function_stubs::$func_name),
            )+
        }
    };
}

impl FFI_AdbcDriver {
    /// Get an empty [Self], but with all functions filled in with stubs.
    ///
    /// Any of the stub functions will simply return [AdbcStatusCode::NotImplemented].
    pub fn empty() -> Self {
        empty_driver!(
            database_init,
            database_new,
            database_set_option,
            database_release,
            connection_commit,
            connection_get_info,
            connection_get_objects,
            connection_get_table_schema,
            connection_get_table_types,
            connection_init,
            connection_new,
            connection_read_partition,
            connection_release,
            connection_rollback,
            connection_set_option,
            statement_bind,
            statement_bind_stream,
            statement_execute_partitions,
            statement_execute_query,
            statement_get_parameter_schema,
            statement_new,
            statement_prepare,
            statement_release,
            statement_set_option,
            statement_set_sql_query,
            statement_set_substrait_plan
        )
    }
}

impl Drop for FFI_AdbcDriver {
    fn drop(&mut self) {
        if let Some(release) = self.release {
            let mut error = FFI_AdbcError::empty();
            let status = unsafe { release(self, &mut error) };
            if status != AdbcStatusCode::Ok {
                panic!("Failed to cleanup driver: {}", unsafe {
                    CStr::from_ptr(error.message).to_string_lossy()
                });
            }
        }
    }
}

unsafe impl Send for FFI_AdbcDriver {}
unsafe impl Sync for FFI_AdbcDriver {}

pub(crate) mod driver_function_stubs {
    use super::*;

    pub(crate) unsafe extern "C" fn database_init(
        _arg1: *mut FFI_AdbcDatabase,
        _arg2: *mut FFI_AdbcError,
    ) -> AdbcStatusCode {
        AdbcStatusCode::NotImplemented
    }

    pub(crate) unsafe extern "C" fn database_new(
        _arg1: *mut FFI_AdbcDatabase,
        _arg2: *mut FFI_AdbcError,
    ) -> AdbcStatusCode {
        AdbcStatusCode::NotImplemented
    }

    pub(crate) unsafe extern "C" fn database_set_option(
        _arg1: *mut FFI_AdbcDatabase,
        _arg2: *const c_char,
        _arg3: *const c_char,
        _arg4: *mut FFI_AdbcError,
    ) -> AdbcStatusCode {
        AdbcStatusCode::NotImplemented
    }

    pub(crate) unsafe extern "C" fn database_release(
        _arg1: *mut FFI_AdbcDatabase,
        _arg2: *mut FFI_AdbcError,
    ) -> AdbcStatusCode {
        AdbcStatusCode::NotImplemented
    }

    pub(crate) unsafe extern "C" fn connection_commit(
        _arg1: *mut FFI_AdbcConnection,
        _arg2: *mut FFI_AdbcError,
    ) -> AdbcStatusCode {
        AdbcStatusCode::NotImplemented
    }

    pub(crate) unsafe extern "C" fn connection_get_info(
        _arg1: *mut FFI_AdbcConnection,
        _arg2: *const u32,
        _arg3: usize,
        _arg4: *mut FFI_ArrowArrayStream,
        _arg5: *mut FFI_AdbcError,
    ) -> AdbcStatusCode {
        AdbcStatusCode::NotImplemented
    }

    pub(crate) unsafe extern "C" fn connection_get_objects(
        _arg1: *mut FFI_AdbcConnection,
        _arg2: AdbcObjectDepth,
        _arg3: *const c_char,
        _arg4: *const c_char,
        _arg5: *const c_char,
        _arg6: *const *const c_char,
        _arg7: *const c_char,
        _arg8: *mut FFI_ArrowArrayStream,
        _arg9: *mut FFI_AdbcError,
    ) -> AdbcStatusCode {
        AdbcStatusCode::NotImplemented
    }

    pub(crate) unsafe extern "C" fn connection_get_table_schema(
        _arg1: *mut FFI_AdbcConnection,
        _arg2: *const c_char,
        _arg3: *const c_char,
        _arg4: *const c_char,
        _arg5: *mut FFI_ArrowSchema,
        _arg6: *mut FFI_AdbcError,
    ) -> AdbcStatusCode {
        AdbcStatusCode::NotImplemented
    }

    pub(crate) unsafe extern "C" fn connection_get_table_types(
        _arg1: *mut FFI_AdbcConnection,
        _arg2: *mut FFI_ArrowArrayStream,
        _arg3: *mut FFI_AdbcError,
    ) -> AdbcStatusCode {
        AdbcStatusCode::NotImplemented
    }

    pub(crate) unsafe extern "C" fn connection_init(
        _arg1: *mut FFI_AdbcConnection,
        _arg2: *mut FFI_AdbcDatabase,
        _arg3: *mut FFI_AdbcError,
    ) -> AdbcStatusCode {
        AdbcStatusCode::NotImplemented
    }

    pub(crate) unsafe extern "C" fn connection_new(
        _arg1: *mut FFI_AdbcConnection,
        _arg2: *mut FFI_AdbcError,
    ) -> AdbcStatusCode {
        AdbcStatusCode::NotImplemented
    }

    pub(crate) unsafe extern "C" fn connection_set_option(
        _arg1: *mut FFI_AdbcConnection,
        _arg2: *const c_char,
        _arg3: *const c_char,
        _arg4: *mut FFI_AdbcError,
    ) -> AdbcStatusCode {
        AdbcStatusCode::NotImplemented
    }

    pub(crate) unsafe extern "C" fn connection_read_partition(
        _arg1: *mut FFI_AdbcConnection,
        _arg2: *const u8,
        _arg3: usize,
        _arg4: *mut FFI_ArrowArrayStream,
        _arg5: *mut FFI_AdbcError,
    ) -> AdbcStatusCode {
        AdbcStatusCode::NotImplemented
    }

    pub(crate) unsafe extern "C" fn connection_release(
        _arg1: *mut FFI_AdbcConnection,
        _arg2: *mut FFI_AdbcError,
    ) -> AdbcStatusCode {
        AdbcStatusCode::NotImplemented
    }

    pub(crate) unsafe extern "C" fn connection_rollback(
        _arg1: *mut FFI_AdbcConnection,
        _arg2: *mut FFI_AdbcError,
    ) -> AdbcStatusCode {
        AdbcStatusCode::NotImplemented
    }

    pub(crate) unsafe extern "C" fn statement_bind(
        _arg1: *mut FFI_AdbcStatement,
        _arg2: *mut FFI_ArrowArray,
        _arg3: *mut FFI_ArrowSchema,
        _arg4: *mut FFI_AdbcError,
    ) -> AdbcStatusCode {
        AdbcStatusCode::NotImplemented
    }

    pub(crate) unsafe extern "C" fn statement_bind_stream(
        _arg1: *mut FFI_AdbcStatement,
        _arg2: *mut FFI_ArrowArrayStream,
        _arg3: *mut FFI_AdbcError,
    ) -> AdbcStatusCode {
        AdbcStatusCode::NotImplemented
    }

    pub(crate) unsafe extern "C" fn statement_execute_query(
        _arg1: *mut FFI_AdbcStatement,
        _arg2: *mut FFI_ArrowArrayStream,
        _arg3: *mut i64,
        _arg4: *mut FFI_AdbcError,
    ) -> AdbcStatusCode {
        AdbcStatusCode::NotImplemented
    }
    pub(crate) unsafe extern "C" fn statement_execute_partitions(
        _arg1: *mut FFI_AdbcStatement,
        _arg2: *mut FFI_ArrowSchema,
        _arg3: *mut FFI_AdbcPartitions,
        _arg4: *mut i64,
        _arg5: *mut FFI_AdbcError,
    ) -> AdbcStatusCode {
        AdbcStatusCode::NotImplemented
    }

    pub(crate) unsafe extern "C" fn statement_get_parameter_schema(
        _arg1: *mut FFI_AdbcStatement,
        _arg2: *mut FFI_ArrowSchema,
        _arg3: *mut FFI_AdbcError,
    ) -> AdbcStatusCode {
        AdbcStatusCode::NotImplemented
    }

    pub(crate) unsafe extern "C" fn statement_new(
        _arg1: *mut FFI_AdbcConnection,
        _arg2: *mut FFI_AdbcStatement,
        _arg3: *mut FFI_AdbcError,
    ) -> AdbcStatusCode {
        AdbcStatusCode::NotImplemented
    }

    pub(crate) unsafe extern "C" fn statement_prepare(
        _arg1: *mut FFI_AdbcStatement,
        _arg2: *mut FFI_AdbcError,
    ) -> AdbcStatusCode {
        AdbcStatusCode::NotImplemented
    }

    pub(crate) unsafe extern "C" fn statement_release(
        _arg1: *mut FFI_AdbcStatement,
        _arg2: *mut FFI_AdbcError,
    ) -> AdbcStatusCode {
        AdbcStatusCode::NotImplemented
    }

    pub(crate) unsafe extern "C" fn statement_set_option(
        _arg1: *mut FFI_AdbcStatement,
        _arg2: *const c_char,
        _arg3: *const c_char,
        _arg4: *mut FFI_AdbcError,
    ) -> AdbcStatusCode {
        AdbcStatusCode::NotImplemented
    }

    pub(crate) unsafe extern "C" fn statement_set_sql_query(
        _arg1: *mut FFI_AdbcStatement,
        _arg2: *const c_char,
        _arg3: *mut FFI_AdbcError,
    ) -> AdbcStatusCode {
        AdbcStatusCode::NotImplemented
    }

    pub(crate) unsafe extern "C" fn statement_set_substrait_plan(
        _arg1: *mut FFI_AdbcStatement,
        _arg2: *const u8,
        _arg3: usize,
        _arg4: *mut FFI_AdbcError,
    ) -> AdbcStatusCode {
        AdbcStatusCode::NotImplemented
    }
}

/// Depth parameter for GetObjects method.
#[derive(Debug)]
#[repr(i32)]
pub enum AdbcObjectDepth {
    /// Metadata on catalogs, schemas, tables, and columns.
    All = 0,
    /// Metadata on catalogs only.
    Catalogs = 1,
    /// Metadata on catalogs and schemas.
    DBSchemas = 2,
    /// Metadata on catalogs, schemas, and tables.
    Tables = 3,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adbc_partitions() {
        let cases: Vec<Vec<Vec<u8>>> =
            vec![vec![], vec![vec![]], vec![vec![0, 1, 2, 3], vec![4, 5, 6]]];

        for case in cases {
            let num_partitions = case.len();
            let expected_partitions = case.clone();

            let mut partitions: FFI_AdbcPartitions = case.into();

            assert_eq!(partitions.num_partitions, num_partitions);
            assert!(!partitions.private_data.is_null());

            for (i, expected_part) in expected_partitions.into_iter().enumerate() {
                let part_length = unsafe { *partitions.partition_lengths.add(i) };
                let part = unsafe {
                    std::slice::from_raw_parts(*partitions.partitions.add(i), part_length)
                };
                assert_eq!(part, &expected_part);
            }

            assert!(partitions.release.is_some());
            let release_func = partitions.release.unwrap();
            unsafe {
                release_func(&mut partitions);
            }

            assert!(partitions.partitions.is_null());
            assert!(partitions.partition_lengths.is_null());
            assert!(partitions.private_data.is_null());
        }
    }
}
