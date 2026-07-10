# Configuration Reference

nasfiles is configured with environment variables.

## Required In Production

| Variable | Description |
|---|---|
| `BASE_URL` | Public URL for the app. Must be `https://...` unless `NASFILES_DEV=1`. |
| `SESSION_SECRET` | At least 64 random bytes as hex, usually generated with `openssl rand -hex 64`. |
| `COMMON_FOLDERS` or `HOME_FOLDER_ROOT` | At least one place where users can browse files. |
| `AUTH_MODE` | `local` or `sso`. Defaults to `sso` when unset. |

## Server

| Variable | Default | Description |
|---|---|---|
| `BIND_ADDR` | `0.0.0.0:8080` | HTTP bind address. |
| `BASE_URL` | `http://localhost:8080` | Public URL used for redirects and share links. |
| `SESSION_SECRET` | dev-only default when `NASFILES_DEV=1` | Hex-encoded signing secret. |
| `DATA_DIR` | `/tmp/nasfiles-data` | Persistent application data, cache, host keys, and default SQLite DB location. |
| `DB_URL` | `sqlite://$DATA_DIR/app.db?mode=rwc` | SQLite or Postgres database URL. |
| `LOG_LEVEL` | `info` | Rust tracing log level. |
| `NASFILES_DEV` | unset | Enables development auth bypass. Do not use in production. |
| `CSP_IMG_SRC_EXTRA` | unset | Extra CSP `img-src` sources, separated by commas or whitespace. |
| `CSP_MEDIA_SRC_EXTRA` | unset | Extra CSP `media-src` sources, separated by commas or whitespace. |

## Reverse Proxy

The Docker Compose examples assume Traefik terminates HTTPS and forwards requests to nasfiles on container port `8080`. Keep `BASE_URL` set to the public HTTPS URL, attach the service to Traefik's external Docker network, and publish the SFTP port separately if you enable SFTP.

Example labels:

```yaml
labels:
  - "traefik.enable=true"
  - "traefik.docker.network=proxy"
  - "traefik.http.routers.nasfiles.rule=Host(`${NASFILES_HOST}`)"
  - "traefik.http.routers.nasfiles.entrypoints=websecure"
  - "traefik.http.routers.nasfiles.tls=true"
  - "traefik.http.routers.nasfiles.tls.certresolver=${TRAEFIK_CERT_RESOLVER:-letsencrypt}"
  - "traefik.http.services.nasfiles.loadbalancer.server.port=8080"
```

## Folder Mounts

| Variable | Description |
|---|---|
| `COMMON_FOLDERS` | JSON map of display name to container path, for example `{"Media":"/mnt/media"}`. |
| `HOME_FOLDER_ROOT` | Optional path for per-user home folders. |
| `SHARE_GROUPS` | Optional JSON map that groups shares in the sidebar, for example `{"Media":["TV Shows","Movies"]}`. |

Folder names in permission variables must match the keys in `COMMON_FOLDERS`.

### Share Groups

`SHARE_GROUPS` tidies the sidebar on installations with many shares by nesting
shares under collapsible group headers. It is a JSON object mapping a group
display name to an ordered list of share keys (the keys from `COMMON_FOLDERS`):

```json
{"Media": ["TV Shows", "Movies", "Short Films"], "Team": ["Documents"]}
```

Notes:

- Grouping is a sidebar overlay only. It does not change access — that stays
  per-share — and a group is never a filesystem location, so files can only be
  created, moved, or copied into the shares inside a group, not into the group.
- A group is shown to a user only when they can see at least one share in it;
  otherwise it is hidden. Shares not listed in any group stay at the top level.
- Groups are sorted alphabetically by name. A share listed under more than one
  group is kept in the first (alphabetically) and a warning is logged. Share
  keys that are not in `COMMON_FOLDERS` are ignored with a warning, and invalid
  JSON disables grouping (the sidebar falls back to a flat list).

## Authentication

| Variable | Default | Description |
|---|---|---|
| `AUTH_MODE` | `sso` | `local` or `sso`. |
| `SETUP_ADMIN_USER` | unset | Local-mode bootstrap admin username. |
| `SETUP_ADMIN_PASSWORD` | unset | Local-mode bootstrap admin password. Must be at least 12 characters. |
| `DISABLE_PASSKEYS` | unset | Set to `true` or `1` to ignore existing passkeys and allow password login. |
| `DISABLE_TOTP` | unset | Set to `true` or `1` to ignore existing TOTP configuration. |
| `TOTP_TRUSTED_DEVICE_TTL_DAYS` | `30` | Trusted-device lifetime for TOTP. |

Local mode has no public registration. Create users from the admin UI.

## OIDC / SSO

| Variable | Default | Description |
|---|---|---|
| `SSO_OIDC_ISSUER_URL` | unset | OIDC issuer URL. |
| `SSO_OIDC_CLIENT_ID` | unset | OIDC client ID. |
| `SSO_OIDC_CLIENT_SECRET` | unset | OIDC client secret. |
| `SSO_OIDC_EXTRA_AUDIENCES` | unset | Extra accepted token audiences, useful for some providers. |
| `SSO_USERNAME_CLAIM` | `preferred_username` | Username claim. |
| `SSO_DISPLAY_NAME_CLAIM` | `name` | Display-name claim. |
| `SSO_PICTURE_CLAIM` | `picture` | Avatar URL claim. |
| `SSO_GROUPS_CLAIM` | `groups` | Group list claim. |
| `SSO_GROUPS_REFRESH_INTERVAL_SECS` | `300` | Interval for refreshing SSO-derived groups and permissions. |

Redirect URI:

```text
https://your-host.example/auth/oidc/callback
```

## Folder Permissions

Default grants for every SSO user:

| Variable | Grants |
|---|---|
| `SSO_DEFAULT_FOLDERS_READ` | Read |
| `SSO_DEFAULT_FOLDERS_WRITE` | Write |
| `SSO_DEFAULT_FOLDERS_SHARE` | Share and read |
| `SSO_DEFAULT_COMMON_FOLDERS` | Read, write, and share |

Group-specific grants:

| Variable pattern | Grants |
|---|---|
| `SSO_GROUP_<GROUP>_FOLDERS_READ` | Read |
| `SSO_GROUP_<GROUP>_FOLDERS_WRITE` | Write |
| `SSO_GROUP_<GROUP>_FOLDERS_SHARE` | Share and read |
| `SSO_GROUP_<GROUP>_COMMON_FOLDERS` | Read, write, and share |

Admin and home-folder controls:

| Variable | Description |
|---|---|
| `SSO_ADMIN_GROUPS` | Comma-separated groups that grant admin access. |
| `SSO_PERSONAL_FOLDER_GROUPS` | Optional comma-separated groups allowed to receive personal folders. |

## Preview And Thumbnailing

| Variable | Default | Description |
|---|---|---|
| `NO_SERVER_SIDE_EXECUTION` | unset | Disables archive extraction, thumbnails, media preview transcoding, and media metadata probing. |
| `THUMBNAIL_CACHE_DIR` | `$DATA_DIR/thumbs` | Thumbnail cache directory. |
| `THUMBNAIL_MAX_SOURCE_FILE_SIZE` | `536870912` | Max source file size for thumbnail generation. |
| `THUMBNAIL_MAX_IMAGE_WIDTH` | `20000` | Max decoded image width. |
| `THUMBNAIL_MAX_IMAGE_HEIGHT` | `20000` | Max decoded image height. |
| `THUMBNAIL_MAX_IMAGE_ALLOC` | `268435456` | Max decoded image allocation. |
| `THUMBNAIL_MAX_CONCURRENT_GENERATIONS` | `2` | Thumbnail worker concurrency. |
| `MEDIA_PREVIEW_MAX_CONCURRENT_TRANSCODES` | `2` | Concurrent media preview transcodes. |

Video thumbnails and media previews depend on `ffmpeg`/`ffprobe`. PDF thumbnails depend on `pdftoppm`. RAW photo thumbnails depend on `dcraw_emu`.

## Search

| Variable | Default | Description |
|---|---|---|
| `SEARCH_MAX_RESULTS` | `100` | Maximum search results returned by one request. |
| `SEARCH_LIVE_ENTRY_BUDGET` | `25000` | Maximum filesystem entries checked during the live search supplement. |
| `SEARCH_LIVE_TIME_BUDGET_MS` | `1500` | Maximum time spent on the live search supplement per request. |
| `SEARCH_REINDEX_INTERVAL_SECS` | `300` | Background metadata index refresh interval. |

## Shares

| Variable | Default | Description |
|---|---|---|
| `SHARE_TOKEN_BYTES` | `24` | Number of random bytes used for share tokens. Clamped to a minimum of `16` (128 bits); lower values are raised to `16`. |

See [Security model](security.md) for how share links, bearer tokens, and file-operation jobs behave around revocation.

## SFTP

| Variable | Default | Description |
|---|---|---|
| `SFTP_ENABLED` | `false` | Enables built-in SFTP service. |
| `SFTP_BIND_ADDR` | `0.0.0.0:2222` | SFTP bind address. |
| `SFTP_HOST_KEY_PATH` | `$DATA_DIR/sftp_host_key` | Persistent SFTP host key path. |

SFTP uses SSH public keys. Users add keys in their profile; admins can create temporary SFTP guests.

## Upload Limits

| Variable | Default | Description |
|---|---|---|
| `MAX_UPLOAD_FILE_SIZE` | `10737418240` | Max single uploaded file, in bytes. |
| `MAX_UPLOAD_REQUEST_SIZE` | `53687091200` | Max upload request, in bytes. |

## S3-Compatible API

The S3 API is always enabled and requires no additional configuration variables. It is available at `{BASE_URL}/s3` and uses AWS Signature Version 4 (SigV4) for authentication.

### Endpoint details

| Property | Value |
|---|---|
| Endpoint | `{BASE_URL}/s3` |
| Region | `us-east-1` (hardcoded; configure this in your S3 client) |
| Addressing style | Path-style only (`/s3/{bucket}/{key}`) |
| Supported operations | ListBuckets, HeadBucket, ListObjectsV2, HeadObject, GetObject (with Range), PutObject, DeleteObject, CreateMultipartUpload, UploadPart, CompleteMultipartUpload, AbortMultipartUpload, ListParts |

### Credential types

**Personal API tokens** — users create tokens from their profile page with a chosen expiry (7 days, 30 days, 90 days, 1 year, or no expiry). The token is scoped to the creating user: each root the user can read appears as a bucket (e.g. `home`, `media`). Permissions are re-checked on every request from the live database — revoking a user's access to a folder takes effect immediately.

**Share credentials** — password-protected shares expose a credential-exchange endpoint:

```
POST /api/public/shares/{share-token}/s3-credentials
Content-Type: application/json

{ "password": "optional-if-share-has-password" }
```

Response:
```json
{
  "access_key": "...",
  "secret_key": "...",
  "expires_at": 1700000000000,
  "endpoint": "https://your-host/s3",
  "bucket": "share",
  "region": "us-east-1"
}
```

Share credentials use `share` as the single bucket name and are scoped to the share path. They expire after one hour or at the share's own expiry, whichever is sooner. Revoking the share immediately blocks further use.

### rclone configuration

```ini
[nasfiles]
type = s3
provider = Other
access_key_id = NASFILES...
secret_access_key = ...
endpoint = https://your-host/s3
force_path_style = true
region = us-east-1
```

Generate credentials from the profile page, or use the share credential exchange endpoint for share-scoped access. `rclone sync --checksum` works because responses include MD5-based ETags.

The `MAX_UPLOAD_FILE_SIZE` limit applies to S3 PutObject. Multipart uploads have no per-part size restriction other than the final assembled file staying within that limit.
