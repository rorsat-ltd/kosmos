# Kosmos - an Iridium SBD Message Broker

Kosmos handles communication with Iridium's Short Burst Data DirectIP service, and turns these messages into HTTP
webhooks.

## Running the server

The server opens a TCP socket to accept connections from Iridium's ground stations.

```shell
kosmos_server --db-url postgres://localhost/kosmos --amqp-addr amqp://localhost --listen_addr [::]:10800
```

Optionally the flag `--nat64-prefix` can be passed to allow the ACL to work correctly on an IPv6 only network.

All configuration options can also be passed as environment variables. Run with `--help` for more information.

## Running the worker

The worker forwards messages to HTTP endpoints, and implements retries on these deliveries. Messages that can't be 
delivered for 24 hours will be marked as failed.

```shell
kosmos_worker --db-url postgres://localhost/kosmos --amqp-addr amqp://localhost
```

All configuration options can also be passed as environment variables. Run with `--help` for more information.

## Webhook format

Messages are sent as HTTP POST JSON with the following format:

```json
{
  "id": "UUID",
  "header": {
    "imei": "IMEI string",
    "cdr_reference": 0,
    "session_status": "normal/too_large/unacceptable_location",
    "mo_msn": 0,
    "mt_msn": 0,
    "time_of_session": "RFC3339 datetime"
  },
  "location_information": {
    "latitude": 0.0,
    "longitude": 0.0,
    "cep_radius": 0,
  },
  "payload": "base64 encoded data"
}
```

## Webhook security

All webhook requests contain a `Kosmos-MAC` header which is a Base64 encoded SHA-256 HMAC over the POST body using
the MAC key from the database.

## Configuring endpoints

Two tables will be created on startup during the database migration process: `targets` and `devices`

### `targets`

This table contains the HTTP endpoints to which webhooks can be delivered. Its fields are:

* `id` - a UUID
* `endpoint` - the HTTP(S) URL to deliver messages to
* `hmac_key` - a binary field containing the HMAC key to use for signing requests

### `devices`

This table maps IMEIs to webhooks. Its fields are:

* `id` - a UUID
* `imei` - text representation of the modem IMEI
* `target` - UUID referencing a target webhook