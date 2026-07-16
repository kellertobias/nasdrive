# SSO / OIDC Setup

NASDrive can use an OpenID Connect provider for login and group-based folder permissions. This is the right setup when users already live in Authentik, Keycloak, Zitadel, Dex, Authelia, Google Workspace, Entra ID, or another OIDC-compatible provider.

## Overview

SSO mode does three things:

- redirects users to your identity provider for login;
- maps OIDC claims to NASDrive users;
- maps OIDC groups to folder permissions and admin access.

Set:

```yaml
AUTH_MODE: "sso"
```

Do not set `NASFILES_DEV=1` in production.

## 1. Create An OIDC Application

In your identity provider, create a confidential OIDC client.

Use these URLs:

```text
Application URL: https://files.example.com
Redirect URI:    https://files.example.com/auth/oidc/callback
Logout URL:      https://files.example.com
```

Enable the scopes needed to receive identity and group data. Most providers need:

```text
openid profile email groups
```

Copy the issuer URL, client ID, and client secret.

## 2. Compose Example

Start from:

[examples/compose.sso.yml](examples/compose.sso.yml)

The example expects Traefik to terminate HTTPS on an external Docker network named `proxy`. Create it once if it does not already exist:

```bash
docker network inspect proxy >/dev/null 2>&1 || docker network create proxy
```

Your Traefik instance should expose a `websecure` entrypoint and a certificate resolver such as `letsencrypt`.

Minimal SSO environment:

```yaml
labels:
  - "traefik.enable=true"
  - "traefik.docker.network=proxy"
  - "traefik.http.routers.nasfiles.rule=Host(`${NASFILES_HOST}`)"
  - "traefik.http.routers.nasfiles.entrypoints=websecure"
  - "traefik.http.routers.nasfiles.tls=true"
  - "traefik.http.routers.nasfiles.tls.certresolver=${TRAEFIK_CERT_RESOLVER:-letsencrypt}"
  - "traefik.http.services.nasfiles.loadbalancer.server.port=8080"
environment:
  BASE_URL: "https://${NASFILES_HOST}"
  AUTH_MODE: "sso"
  SESSION_SECRET: "${SESSION_SECRET}"

  SSO_OIDC_ISSUER_URL: "https://idp.example.com/application/o/nasfiles/"
  SSO_OIDC_CLIENT_ID: "${SSO_OIDC_CLIENT_ID}"
  SSO_OIDC_CLIENT_SECRET: "${SSO_OIDC_CLIENT_SECRET}"

  COMMON_FOLDERS: '{"Media":"/mnt/media","Documents":"/mnt/documents"}'
  SSO_DEFAULT_FOLDERS_READ: "Media"
  SSO_GROUP_EDITORS_COMMON_FOLDERS: "Documents"
  SSO_ADMIN_GROUPS: "admins"
```

Create `.env`:

```dotenv
NASFILES_HOST=files.example.com
TRAEFIK_CERT_RESOLVER=letsencrypt
SESSION_SECRET=replace-with-output-of-openssl-rand-hex-64
SSO_OIDC_CLIENT_ID=replace-me
SSO_OIDC_CLIENT_SECRET=replace-me
```

Generate the session secret:

```bash
openssl rand -hex 64
```

Start:

```bash
docker compose up -d --build
```

## 3. Claim Mapping

Defaults:

| Variable | Default | Meaning |
|---|---|---|
| `SSO_USERNAME_CLAIM` | `preferred_username` | Stable username shown in NASDrive |
| `SSO_DISPLAY_NAME_CLAIM` | `name` | Human-readable display name |
| `SSO_PICTURE_CLAIM` | `picture` | Avatar URL |
| `SSO_GROUPS_CLAIM` | `groups` | Group list used for permissions |

If your provider uses different names, override them:

```yaml
environment:
  SSO_USERNAME_CLAIM: "email"
  SSO_DISPLAY_NAME_CLAIM: "name"
  SSO_GROUPS_CLAIM: "roles"
```

## 4. Folder Permissions

Declare the host folders:

```yaml
COMMON_FOLDERS: '{"Media":"/mnt/media","Documents":"/mnt/documents","Projects":"/mnt/projects"}'
```

Then grant access.

Everyone can read `Media`:

```yaml
SSO_DEFAULT_FOLDERS_READ: "Media"
```

Everyone can read, write, and share `Media`:

```yaml
SSO_DEFAULT_COMMON_FOLDERS: "Media"
```

Members of group `EDITORS` get full access to `Documents` and `Projects`:

```yaml
SSO_GROUP_EDITORS_COMMON_FOLDERS: "Documents,Projects"
```

Members of group `REVIEWERS` can read and share `Documents`, but not write:

```yaml
SSO_GROUP_REVIEWERS_FOLDERS_READ: "Documents"
SSO_GROUP_REVIEWERS_FOLDERS_SHARE: "Documents"
```

Folder capability variables:

| Variable pattern | Grants |
|---|---|
| `SSO_DEFAULT_FOLDERS_READ` | Read access for all users |
| `SSO_DEFAULT_FOLDERS_WRITE` | Write access for all users |
| `SSO_DEFAULT_FOLDERS_SHARE` | Share access for all users; also implies read |
| `SSO_DEFAULT_COMMON_FOLDERS` | Read, write, and share for all users |
| `SSO_GROUP_<GROUP>_FOLDERS_READ` | Read for members of `<GROUP>` |
| `SSO_GROUP_<GROUP>_FOLDERS_WRITE` | Write for members of `<GROUP>` |
| `SSO_GROUP_<GROUP>_FOLDERS_SHARE` | Share for members of `<GROUP>`; also implies read |
| `SSO_GROUP_<GROUP>_COMMON_FOLDERS` | Read, write, and share for members of `<GROUP>` |

Group names in environment variables must be uppercase-safe shell identifiers. If your provider group is `nas editors`, prefer an IdP role or group slug such as `EDITORS`.

## 5. Admin Access

Grant admin access to one or more groups:

```yaml
SSO_ADMIN_GROUPS: "admins,nasfiles-admins"
```

Admins can view shares, audit access, manage local users in local mode, and create temporary SFTP guests.

## 6. Personal Folders

Set a host path:

```yaml
HOME_FOLDER_ROOT: "/homes"
```

By default, SSO users can get personal folders when `HOME_FOLDER_ROOT` is set. To restrict this to certain groups:

```yaml
SSO_PERSONAL_FOLDER_GROUPS: "staff,contractors"
```

## 7. Provider Notes

### Authentik

- Application type: OIDC Provider
- Redirect URI: `https://files.example.com/auth/oidc/callback`
- Include `groups` in the token or userinfo response.

### Keycloak

- Client type: confidential
- Valid redirect URI: `https://files.example.com/auth/oidc/callback`
- Add a group membership mapper if group claims are not present by default.

### Zitadel

Some Zitadel setups include a project ID in token audiences. If needed, add:

```yaml
SSO_OIDC_EXTRA_AUDIENCES: "your-project-id"
```

## Troubleshooting

If login succeeds but no folders appear:

- confirm the user has the expected group claim;
- confirm the folder names in permission variables exactly match `COMMON_FOLDERS` keys;
- check container logs for OIDC and permission mapping warnings.

If redirects fail:

- confirm `BASE_URL` is the public HTTPS URL;
- confirm the IdP redirect URI is exactly `BASE_URL + /auth/oidc/callback`;
- confirm the Traefik router rule uses the same host as `BASE_URL`;
- confirm reverse proxy headers preserve the original host and scheme.
