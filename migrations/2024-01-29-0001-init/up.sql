create type session_status as enum (
    'successful',
    'successful_too_large',
    'successful_unacceptable_location',
    'timeout',
    'too_large',
    'rf_link_lost',
    'protocol_anomaly',
    'imei_blocked'
);

create type processing_status as enum (
    'received',
    'done',
    'failed'
);

create table mo_messages (
    id uuid primary key,
    cdr_reference int4 not null,
    imei char(15) not null,
    session_status session_status not null,
    mo_msn int2 not null,
    mt_msn int2 not null,
    time_of_session timestamp not null,
    latitude float4 null,
    longitude float4 null,
    cep_radius int4 null,
    data bytea null,
    processing_status processing_status not null,
    received timestamp not null,
    last_processed timestamp null
);

create type message_status as enum (
    'invalid_imei',
    'payload_size_exceeded',
    'message_queue_full',
    'resources_unavailable'
);

create table mt_messages (
    id uuid primary key,
    imei char(15) not null,
    priority smallint not null,
    data bytea null,
    message_status message_status null,
    processing_status processing_status not null,
    last_processed timestamp not null
)