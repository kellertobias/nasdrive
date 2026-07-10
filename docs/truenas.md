# TrueNAS Example

nasfiles works well on TrueNAS because it does not need to own your storage. You can mount existing datasets into the container and expose them through a browser UI while SMB/NFS shares keep working.

This guide uses TrueNAS SCALE terminology. Exact labels may vary between TrueNAS releases.

## Example Dataset Layout

Create or reuse datasets like:

```text
/mnt/tank/media
/mnt/tank/documents
/mnt/tank/projects
/mnt/tank/apps/nasfiles
```

Recommended use:

| Dataset | Mounted as | Purpose |
|---|---|---|
| `/mnt/tank/apps/nasfiles` | `/data` | nasfiles database, sessions, thumbnails, SFTP host key |
| `/mnt/tank/media` | `/mnt/media` | read-only media library |
| `/mnt/tank/documents` | `/mnt/documents` | read/write documents |
| `/mnt/tank/projects` | `/mnt/projects` | read/write project files |

Keep `/data` persistent. If you delete it, users, shares, SFTP guests, thumbnail cache, and local auth state are lost.

## Option A: TrueNAS Custom App

In TrueNAS SCALE:

1. Open **Apps**.
2. Choose **Discover Apps** or **Custom App**.
3. Create a custom container app for nasfiles.
4. Set the web port to `8080`.
5. Add host-path storage mounts.
6. Add environment variables.
7. Deploy the app.

### Container Image

If you build locally from this repository, use your own image tag after pushing it to a registry:

```text
registry.example.com/nasfiles:latest
```

If you deploy from a checkout through Compose support, use the compose example below instead.

### Ports And HTTPS

For production, put the web UI behind Traefik and use HTTPS. If Traefik runs as a Docker app on the same host, route it to the nasfiles app on port `8080`. If Traefik runs somewhere else on your network, point a router or service at the TrueNAS host and port `8080`.

Expose internally or on the LAN:

```text
8080/tcp -> 8080
2222/tcp -> 2222  optional, only if using SFTP
```

### Storage Mounts

Add:

```text
/mnt/tank/apps/nasfiles -> /data
/mnt/tank/media         -> /mnt/media       read-only
/mnt/tank/documents     -> /mnt/documents   read/write
/mnt/tank/projects      -> /mnt/projects    read/write
```

### Local-User Environment

For a plain local-user setup:

```text
BIND_ADDR=0.0.0.0:8080
BASE_URL=https://files.example.com
AUTH_MODE=local
SESSION_SECRET=<openssl rand -hex 64>
DATA_DIR=/data
DB_URL=sqlite:///data/nasfiles.db?mode=rwc
COMMON_FOLDERS={"Media":"/mnt/media","Documents":"/mnt/documents","Projects":"/mnt/projects"}
SETUP_ADMIN_USER=admin
SETUP_ADMIN_PASSWORD=<long initial password>
SFTP_ENABLED=true
SFTP_BIND_ADDR=0.0.0.0:2222
RUST_LOG=info
```

After first login, create named users from the admin UI.

## Option B: Compose App

If your TrueNAS setup supports deploying Compose apps, adapt this:

```yaml
services:
  nasfiles:
    image: registry.example.com/nasfiles:latest
    restart: unless-stopped
    ports:
      - "2222:2222"
    labels:
      - "traefik.enable=true"
      - "traefik.docker.network=proxy"
      - "traefik.http.routers.nasfiles.rule=Host(`${NASFILES_HOST}`)"
      - "traefik.http.routers.nasfiles.entrypoints=websecure"
      - "traefik.http.routers.nasfiles.tls=true"
      - "traefik.http.routers.nasfiles.tls.certresolver=${TRAEFIK_CERT_RESOLVER:-letsencrypt}"
      - "traefik.http.services.nasfiles.loadbalancer.server.port=8080"
    environment:
      BIND_ADDR: "0.0.0.0:8080"
      BASE_URL: "https://${NASFILES_HOST}"
      AUTH_MODE: "local"
      SESSION_SECRET: "${SESSION_SECRET}"
      DATA_DIR: "/data"
      DB_URL: "sqlite:///data/nasfiles.db?mode=rwc"
      COMMON_FOLDERS: '{"Media":"/mnt/media","Documents":"/mnt/documents","Projects":"/mnt/projects"}'
      SETUP_ADMIN_USER: "admin"
      SETUP_ADMIN_PASSWORD: "${SETUP_ADMIN_PASSWORD}"
      SFTP_ENABLED: "true"
      SFTP_BIND_ADDR: "0.0.0.0:2222"
    volumes:
      - /mnt/tank/apps/nasfiles:/data
      - /mnt/tank/media:/mnt/media:ro
      - /mnt/tank/documents:/mnt/documents
      - /mnt/tank/projects:/mnt/projects
    networks:
      - proxy

networks:
  proxy:
    external: true
```

Create the `.env` values:

```dotenv
NASFILES_HOST=files.example.com
TRAEFIK_CERT_RESOLVER=letsencrypt
SESSION_SECRET=replace-with-openssl-rand-hex-64
SETUP_ADMIN_PASSWORD=replace-with-a-long-initial-password
```

Create the shared Traefik network once if it does not exist:

```bash
docker network inspect proxy >/dev/null 2>&1 || docker network create proxy
```

SFTP is still exposed directly on `2222` in this example. The Traefik labels above configure the HTTPS web UI only.

## SSO On TrueNAS

If you already run Authentik, Keycloak, Authelia, or another IdP, use SSO mode:

```yaml
environment:
  BASE_URL: "https://files.example.com"
  AUTH_MODE: "sso"
  SESSION_SECRET: "${SESSION_SECRET}"
  SSO_OIDC_ISSUER_URL: "https://auth.example.com/application/o/nasfiles/"
  SSO_OIDC_CLIENT_ID: "${SSO_OIDC_CLIENT_ID}"
  SSO_OIDC_CLIENT_SECRET: "${SSO_OIDC_CLIENT_SECRET}"
  COMMON_FOLDERS: '{"Media":"/mnt/media","Documents":"/mnt/documents","Projects":"/mnt/projects"}'
  SSO_DEFAULT_FOLDERS_READ: "Media"
  SSO_GROUP_FAMILY_COMMON_FOLDERS: "Documents"
  SSO_GROUP_ADMINS_COMMON_FOLDERS: "Media,Documents,Projects"
  SSO_ADMIN_GROUPS: "ADMINS"
```

Set your OIDC redirect URI to:

```text
https://files.example.com/auth/oidc/callback
```

Put nasfiles behind Traefik when using SSO. `BASE_URL` should be the final public HTTPS URL that users see.

## Permissions

The container user must be able to read/write the mounted datasets according to the permissions you want nasfiles to have.

Typical approaches:

- run the app with a TrueNAS app user that has ACL entries on the datasets;
- mount read-only datasets with `:ro`;
- create a dedicated `nasfiles` user/group and grant it access to only the datasets exposed in `COMMON_FOLDERS`.

Remember that nasfiles cannot write where the container does not have filesystem permission, even if the UI grants write access.

## SMB/NFS Compatibility

nasfiles operates on the same files as SMB and NFS. That is intentional.

Practical tips:

- Use normal dataset snapshots for rollback.
- Avoid editing the same file concurrently through SMB and the web UI.
- Keep backup jobs pointed at the datasets, not at `/data`.
- Do not expose `/data` as a user share; it is application state.

## Troubleshooting

If the app opens but folders are empty:

- check `COMMON_FOLDERS` JSON syntax;
- confirm host paths are mounted into the container;
- confirm the container user has permission to read those paths.

If thumbnails are missing:

- confirm the image includes `ffmpeg` for video thumbnails;
- confirm `pdftoppm` is available for PDF thumbnails;
- confirm `dcraw_emu` is available for RAW photo thumbnails;
- check `DATA_DIR` is writable for thumbnail cache storage.

If SFTP does not work:

- expose port `2222`;
- set `SFTP_ENABLED=true`;
- add an SSH public key to the user profile or create a temporary SFTP guest;
- connect with `sftp -P 2222 <username>@<truenas-host>`.
