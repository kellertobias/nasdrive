# Docker Compose Setup

This guide runs NASDrive with local users. It is the simplest setup for a home NAS, lab server, or small team share.

## What You Get

- Web UI on `https://files.example.com` through Traefik
- SQLite metadata stored in a Docker volume
- One mounted folder named `Files`
- Local admin bootstrap user
- Optional SFTP service on port `2222`

The compose example assumes Traefik is already running on the same Docker host with:

- an external Docker network named `proxy`;
- a HTTPS entrypoint named `websecure`;
- a certificate resolver named `letsencrypt`, or another resolver set through `TRAEFIK_CERT_RESOLVER`.

Create the shared network once if it does not exist yet:

```bash
docker network inspect proxy >/dev/null 2>&1 || docker network create proxy
```

## 1. Prepare Folders

From the project checkout:

```bash
mkdir -p files data
cp docs/examples/compose.local.yml docker-compose.yml
```

Put a few test files in `./files`, or replace the bind mount with the real folder you want to expose.

## 2. Create Secrets

Create a `.env` file next to `docker-compose.yml`:

```dotenv
NASFILES_HOST=files.example.com
TRAEFIK_CERT_RESOLVER=letsencrypt
SESSION_SECRET=replace-with-128-hex-characters
SETUP_ADMIN_PASSWORD=replace-with-a-long-initial-password
```

Generate a valid session secret with:

```bash
openssl rand -hex 64
```

Use a setup password with at least 12 characters. After the first login, change it in the UI or create separate users and stop using the bootstrap password operationally.

## 3. Start NASDrive

```bash
docker compose up -d --build
```

Open:

```text
https://files.example.com
```

Sign in with:

```text
username: admin
password: the value of SETUP_ADMIN_PASSWORD
```

## 4. Configure Users

Open the admin screen and create users. For each user you can assign:

- read access;
- write access;
- share-link permission;
- home-folder access if `HOME_FOLDER_ROOT` is configured;
- admin privileges.

The setup admin automatically gets full access to all configured common folders.

## 5. Mount Real Folders

Edit the compose file:

```yaml
environment:
  COMMON_FOLDERS: '{"Media":"/files/media","Documents":"/files/documents"}'
volumes:
  - /mnt/storage/media:/files/media
  - /mnt/storage/documents:/files/documents
```

Use `:ro` on a volume if NASDrive should not write to it:

```yaml
volumes:
  - /mnt/archive:/files/archive:ro
```

## 6. Optional Personal Home Folders

Set a root path for per-user folders:

```yaml
environment:
  HOME_FOLDER_ROOT: "/homes"
volumes:
  - /mnt/nasfiles-homes:/homes
```

When a user has home access, NASDrive creates and exposes their personal folder.

## 7. Optional SFTP

The local compose example enables SFTP:

```yaml
ports:
  - "2222:2222"
environment:
  SFTP_ENABLED: "true"
  SFTP_BIND_ADDR: "0.0.0.0:2222"
```

Users add their own SSH public keys in their profile. Admins can also create temporary SFTP guests scoped to a folder.

Connect with:

```bash
sftp -P 2222 admin@localhost
```

SFTP authentication is public-key only.

## 8. Run Behind A Reverse Proxy

The example compose file is already configured for Traefik. It does not publish the HTTP port directly; instead, Traefik discovers NASDrive through Docker labels:

```yaml
labels:
  - "traefik.enable=true"
  - "traefik.docker.network=proxy"
  - "traefik.http.routers.nasfiles.rule=Host(`${NASFILES_HOST}`)"
  - "traefik.http.routers.nasfiles.entrypoints=websecure"
  - "traefik.http.routers.nasfiles.tls=true"
  - "traefik.http.routers.nasfiles.tls.certresolver=${TRAEFIK_CERT_RESOLVER:-letsencrypt}"
  - "traefik.http.services.nasfiles.loadbalancer.server.port=8080"
networks:
  - proxy
```

`BASE_URL` must match the public HTTPS URL:

```yaml
environment:
  BASE_URL: "https://${NASFILES_HOST}"
```

For quick local testing without Traefik, temporarily publish the HTTP port and set `BASE_URL` to `http://localhost:8080`. Do not use that setup for internet-facing deployments.

Do not use `NASFILES_DEV=1` for production. Dev mode bypasses authentication and is only for local demos.

## Updating

From a source checkout:

```bash
git pull
docker compose up -d --build
```

With a published image:

```bash
docker compose pull
docker compose up -d
```
