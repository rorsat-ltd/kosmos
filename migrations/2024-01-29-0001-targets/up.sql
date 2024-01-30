create table targets (
    id uuid primary key,
    hmac_key bytea not null,
    endpoint varchar not null
);

create table devices (
    id uuid primary key,
    imei char(15) not null,
    target uuid references targets(id) not null
)