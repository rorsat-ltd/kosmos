// @generated automatically by Diesel CLI.

pub mod sql_types {
    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "message_status"))]
    pub struct MessageStatus;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "processing_status"))]
    pub struct ProcessingStatus;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "session_status"))]
    pub struct SessionStatus;
}

diesel::table! {
    devices (id) {
        id -> Uuid,
        #[max_length = 15]
        imei -> Bpchar,
        target -> Uuid,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::SessionStatus;
    use super::sql_types::ProcessingStatus;

    mo_messages (id) {
        id -> Uuid,
        cdr_reference -> Int4,
        #[max_length = 15]
        imei -> Bpchar,
        session_status -> SessionStatus,
        mo_msn -> Int2,
        mt_msn -> Int2,
        time_of_session -> Timestamp,
        latitude -> Nullable<Float4>,
        longitude -> Nullable<Float4>,
        cep_radius -> Nullable<Int4>,
        data -> Nullable<Bytea>,
        processing_status -> ProcessingStatus,
        received -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::MessageStatus;
    use super::sql_types::ProcessingStatus;

    mt_messages (id) {
        id -> Uuid,
        #[max_length = 15]
        imei -> Bpchar,
        priority -> Int2,
        data -> Bytea,
        message_status -> Nullable<MessageStatus>,
        processing_status -> ProcessingStatus,
        received -> Timestamp,
        target -> Uuid,
    }
}

diesel::table! {
    targets (id) {
        id -> Uuid,
        hmac_key -> Bytea,
        endpoint -> Varchar,
    }
}

diesel::joinable!(devices -> targets (target));
diesel::joinable!(mt_messages -> targets (target));

diesel::allow_tables_to_appear_in_same_query!(
    devices,
    mo_messages,
    mt_messages,
    targets,
);
