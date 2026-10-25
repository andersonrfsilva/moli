use super::*;
use crate::context_bootstrap::indexed_db::schedule_indexed_db_transaction_deactivation_after_microtask_checkpoint;
use crate::context_bootstrap::indexed_db::{
    INDEXED_DB_DATABASE_UPGRADE_TRANSACTION_SLOT, INDEXED_DB_TRANSACTION_FINISHED_SLOT,
    object_store_info_from_database_metadata,
};
use crate::webidl;

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "IDBTransactionMode")]
enum TransactionModeWebIdl {
    Readonly,
    Readwrite,
    Versionchange,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IDBDatabase.transaction")]
struct IdbDatabaseTransactionArgs {
    #[webidl(required, name = "storeNames", converter = "raw")]
    store_names: names::TransactionStoreNames,
    #[webidl(converter = "enum", default = TransactionModeWebIdl::Readonly)]
    mode: TransactionModeWebIdl,
}

pub(in crate::context_bootstrap::indexed_db) fn idb_database_transaction_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<IdbDatabaseTransactionArgs>(scope, &args) else {
        return;
    };
    let database = args.this();
    let Some(handle) = database_handle_from_value(scope, database.into()) else {
        rv.set_undefined();
        return;
    };
    // A live upgrade includes its inactive and committing states. Checking the
    // connection's own transaction also works when this method is borrowed
    // from another realm.
    if let Some(upgrade) = object_property_as_object(
        scope,
        database,
        INDEXED_DB_DATABASE_UPGRADE_TRANSACTION_SLOT,
    ) && !object_bool_property(scope, upgrade, INDEXED_DB_TRANSACTION_FINISHED_SLOT)
        .unwrap_or(false)
    {
        let error = dom_exception_value(
            scope,
            "The database is running a version change transaction.",
            "InvalidStateError",
        );
        scope.throw_exception(error);
        return;
    }
    if object_bool_property(scope, database, INDEXED_DB_DATABASE_CLOSED_SLOT).unwrap_or(false) {
        let error = dom_exception_value(
            scope,
            "The database connection is closing.",
            "InvalidStateError",
        );
        scope.throw_exception(error);
        return;
    }
    let mut store_names = parsed.store_names.0;
    store_names.sort_unstable();
    store_names.dedup();
    // Validate before queuing a readwrite transaction as well: its backend
    // handle may only be allocated after an earlier transaction finishes.
    if store_names
        .iter()
        .any(|name| object_store_info_from_database_metadata(scope, database, name).is_none())
    {
        let error = dom_exception_value(
            scope,
            "An object store in the transaction scope was not found.",
            "NotFoundError",
        );
        scope.throw_exception(error);
        return;
    }
    if store_names.is_empty() {
        let error = dom_exception_value(
            scope,
            "The transaction scope is empty.",
            "InvalidAccessError",
        );
        scope.throw_exception(error);
        return;
    }
    let mode = match parsed.mode {
        TransactionModeWebIdl::Readonly => TransactionMode::ReadOnly,
        TransactionModeWebIdl::Readwrite => TransactionMode::ReadWrite,
        // This is a valid IDL enum value, so its rejection comes after the
        // connection and scope checks, unlike an unrecognized enum string.
        TransactionModeWebIdl::Versionchange => {
            throw_type_error(scope, "Failed to execute 'transaction': unsupported mode.");
            return;
        }
    };
    if mode == TransactionMode::ReadWrite {
        let db_key = object_string_property(scope, database, INDEXED_DB_DATABASE_KEY_SLOT)
            .unwrap_or_default();
        if !has_unfinished_readwrite_transaction_for_db(scope, &db_key) {
            match with_indexed_db_manager(scope, |manager| {
                manager.begin_transaction(handle, &store_names, mode)
            }) {
                Ok(transaction_handle) => {
                    let Some(transaction) = create_transaction_object(
                        scope,
                        database,
                        Some(transaction_handle),
                        mode,
                        &store_names,
                    ) else {
                        rv.set_undefined();
                        return;
                    };
                    register_readwrite_transaction(scope, transaction);
                    schedule_indexed_db_transaction_deactivation_after_microtask_checkpoint(
                        scope,
                        transaction,
                    );
                    rv.set(transaction.into());
                }
                Err(error) => {
                    let error = request_error_object(scope, &error);
                    scope.throw_exception(error);
                }
            }
            return;
        }
        let Some(transaction) =
            create_transaction_object(scope, database, None, mode, &store_names)
        else {
            rv.set_undefined();
            return;
        };
        register_readwrite_transaction(scope, transaction);
        schedule_indexed_db_transaction_deactivation_after_microtask_checkpoint(scope, transaction);
        rv.set(transaction.into());
        return;
    }
    match with_indexed_db_manager(scope, |manager| {
        manager.begin_transaction(handle, &store_names, mode)
    }) {
        Ok(transaction_handle) => {
            let Some(transaction) = create_transaction_object(
                scope,
                database,
                Some(transaction_handle),
                mode,
                &store_names,
            ) else {
                rv.set_undefined();
                return;
            };
            schedule_indexed_db_transaction_deactivation_after_microtask_checkpoint(
                scope,
                transaction,
            );
            rv.set(transaction.into());
        }
        Err(error) => {
            let error = request_error_object(scope, &error);
            scope.throw_exception(error);
        }
    }
}

pub(in crate::context_bootstrap::indexed_db) fn idb_database_close_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let database = args.this();
    close_indexed_db_database_connection(scope, database);
    rv.set_undefined();
}

pub(in crate::context_bootstrap::indexed_db) fn close_indexed_db_database_connection(
    scope: &mut v8::PinScope<'_, '_>,
    database: v8::Local<'_, v8::Object>,
) {
    if object_bool_property(scope, database, INDEXED_DB_DATABASE_CLOSED_SLOT).unwrap_or(false) {
        return;
    }
    set_indexed_db_slot_value(
        scope,
        database,
        INDEXED_DB_DATABASE_CLOSED_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    let coordinated = database_handle_from_value(scope, database.into())
        .is_some_and(|handle| unregister_open_database_connection(scope, handle, database));
    if let Some(handle) = database_handle_from_value(scope, database.into()) {
        let _ = with_indexed_db_manager(scope, |manager| manager.close_database(handle));
    }
    if !coordinated {
        enqueue_drain_blocked_open_requests_task(scope);
    }
}
