<p align="center">
  <img src="web/public/favicon.svg" alt="NASDrive logo" width="96" height="96">
</p>

<h1 align="center">NASDrive by Tobisk</h1>

NASDrive is a small, fast cloud solution & web file manager for your home server or small business file share.

It is not trying to replace full personal-cloud platforms like ownCloud or Nextcloud at least not on its own.

NASDrive focuses on the part that matters for many NAS-style setups: a usable web-based file manager with cloud-like sharing, previews, uploads, and optional SFTP and secure guest access, all with a small footprint.

The underlying filesystem stays the source of truth. Your files remain normal files and folders, so the same content can still be served by SMB, NFS, SFTP, backup jobs, media tools, shell scripts, and the other services already running on your NAS.

## Why Try It

- User Experience comes first. We have everything you expect from a connected file manager in a very usable and user friendly way:
  - Preview images, video, audio, PDFs, Markdown, code, and archives in the browser
  - Switch between grid, list, and macOS-style column browsing
  - Manage your files with drag and drop, even across multiple browser windows
  - Sharing via public Link, with or without password or expiration date (and even with temporary SSH keys via SFTP), including access audit
  - Full virtual SFTP implementation. You only see the folders you have access to and never accidentally give users full SSH access.
  - Folder Readmes (just create a README.md in the folder)
- Great Operations Setup:
  - The file system is the source of truth. You mount the folders, everything else is done by the file system. No Duplication or outdated indexes.
  - Fully Local and Fully SSO/ OIDC support, including permissions to the mounts
  - Fully configurable with environment variables
  - single container deployment with very small resource footprint
- Focus on Security:
  - Full Audit logs
  - SFTP server for secure access
  - Support for PassKeys (FIDO2) and Password + TOTP. Trusted browser support.

We are not trying to replace Nextcloud, Dropbox, or a full document collaboration suite. We focus on the part the these originally became successful for - file management and sharing, but that with dedication and a way smaller footprint.

![NASDrive file manager showing thumbnails, folder navigation, and details](docs/assets/screenshots/main-app.png)

## Quick Start

The fastest production-like setup is Docker Compose with local users:

```bash
git clone git@github.com:kellertobias/tobisk-nasfiles.git nasfiles
cd nasfiles
cp docs/examples/compose.local.yml docker-compose.yml
mkdir -p ./data ./files
printf "NASFILES_HOST=%s\n" "files.example.com" > .env
printf "TRAEFIK_CERT_RESOLVER=%s\n" "letsencrypt" >> .env
printf "SESSION_SECRET=%s\n" "$(openssl rand -hex 64)" >> .env
printf "SETUP_ADMIN_PASSWORD=%s\n" "change-this-long-password" >> .env
docker network inspect proxy >/dev/null 2>&1 || docker network create proxy
docker compose up -d --build
```

Open `https://files.example.com`, sign in with the setup admin configured in the compose file, then create real users from the admin screen. The example assumes Traefik is already running with a `websecure` entrypoint and access to the external Docker network named `proxy`.

For complete setup guides, see:

- [Docker Compose with local users](docs/docker-compose.md)
- [Docker Compose with SSO/OIDC](docs/sso.md)
- [TrueNAS setup example](docs/truenas.md)
- [Configuration reference](docs/configuration.md)
- [Security model & operational notes](docs/security.md)

### Container images

Prebuilt images are published to the GitHub Container Registry:

- **[`ghcr.io/kellertobias/nasdrive`](https://github.com/kellertobias/tobisk-nasfiles/pkgs/container/nasdrive)** — `:latest` tracks `main`, and each [release](https://github.com/kellertobias/tobisk-nasfiles/releases) publishes immutable `:MAJOR.MINOR.PATCH` and `:MAJOR.MINOR` tags.

```bash
docker pull ghcr.io/kellertobias/nasdrive:latest
```

The Forgejo pipeline continues to publish the fixed `nasfiles` image name used by the hosted deployment; that name is unchanged.

## Features

### File Browsing

- Grid, list, and column views
- Folder tree and breadcrumbs
- Drag-and-drop uploads
- Folder uploads where supported by the browser
- Move, copy, rename, delete, and create-folder operations
- Streaming ZIP downloads for selected files or folders

![NASDrive macOS-style column view browsing files](docs/assets/screenshots/column-view.png)

![NASDrive drag and drop across browser windows](docs/assets/screenshots/drag-n-drop-support.png)

### Previews

- Image thumbnails and image preview
- Video thumbnails, streaming, and range requests
- Audio playback with embedded cover art where available
- PDF thumbnails and preview
- Markdown and code previews
- Archive browsing and extraction when server-side execution is enabled

![NASDrive media previews for documents, music, and video](docs/assets/screenshots/media-preview.png)

### Sharing And Access

- Public share links
- Password-protected guest shares
- Expiring links
- Optional upload permission on shares
- Share audit and admin visibility
- Temporary SFTP guests with folder-scoped access
- S3-compatible API for programmatic access via rclone, the AWS CLI, Cyberduck, and other S3-capable tools

### S3-Compatible API

Any user can create long-lived personal API tokens from their profile page and use them to access their files over an S3-compatible endpoint (`/s3/`). Each accessible root becomes a bucket (`home`, `media`, …). Writeable password-protected shares additionally expose a credential-exchange endpoint so the share password can be traded for short-lived S3 credentials scoped to the share path.

Tools that work out of the box:

```ini
# rclone — rclone ls nasdrive:home/
[nasdrive]
type = s3
provider = Other
access_key_id = <your_access_key>
secret_access_key = <your_secret_key>
endpoint = https://your-host/s3
force_path_style = true
region = us-east-1
```

The S3 endpoint implements `ListBuckets`, `HeadBucket`, `ListObjectsV2`, `HeadObject`, `GetObject` (with range requests), `PutObject`, `DeleteObject`, and multipart upload (`CreateMultipartUpload`, `UploadPart`, `CompleteMultipartUpload`, `AbortMultipartUpload`, `ListParts`). Responses include MD5-based ETags, enabling `rclone sync --checksum`.

### Authentication

- Local users with setup-admin bootstrap
- Passkey support
- TOTP support with trusted devices
- OIDC/SSO mode for identity-provider-backed deployments
- Group-to-folder permission mapping for SSO users

![NASDrive user administration screen](docs/assets/screenshots/user-admin.png)

## How It Works

NASDrive mounts one or more host paths as named roots:

```text
COMMON_FOLDERS='{"Media":"/srv/media","Documents":"/srv/docs"}'
```

The server canonicalizes paths and keeps every operation inside the configured roots. The browser UI talks to the Rust API. Metadata, users, shares, SFTP guests, and thumbnail cache data live under `DATA_DIR` by default.

On installations with many shares you can tidy the sidebar by grouping shares under collapsible headers with `SHARE_GROUPS` (for example `{"Media":["TV Shows","Movies"]}`). Grouping is purely a sidebar overlay — it never changes access, and a group is shown only when a user can see at least one share inside it. See [Configuration reference](docs/configuration.md) for details.

```text
Browser UI
   |
   v
NASDrive server
   |-- SQLite/Postgres metadata
   |-- thumbnail/cache data
   `-- configured host folders
```

### Code Safaris — guided tours of the source

Want the longer version? **[Take a code safari →](https://kellertobias.github.io/tobisk-nasfiles/)**

Code safaris are guided walkthroughs that follow real control flow through the
codebase, in a read-only IDE-style viewer — file tree on the left, the actual
source in the middle, the narrative on the right. There are five:

| Tour | What it follows |
| --- | --- |
| **Authentication** | A login from the browser form through rate limiting, TOTP, and session creation — then how every later request re-validates the user against the database. |
| **Share management** | The life of a share link: creation, token generation, a stranger redeeming it, and the two very different ways a share dies. |
| **File copy & move** | A drag-and-drop turned into a durable, resumable, cancellable background job, with progress derived rather than counted. |
| **SFTP server** | Host keys, public-key-only auth, the synthetic virtual root, and per-operation permission revalidation. |
| **S3-compatible API** | SigV4 verification, bucket-to-root mapping, ListObjectsV2, PutObject, and multipart uploads. |

The narrative lives in [`.tour/`](.tour/index.md) plus `@tour` comments in the
source itself, so it moves with the code. To read it locally:

```bash
npx @tobisk/codesafari dev      # http://localhost:4317
npx @tobisk/codesafari validate # check the tours still resolve
```

The published site is rebuilt by
[`.github/workflows/code-safari.yml`](.github/workflows/code-safari.yml) on every
push to `main`.

## Deployment Notes

- Put NASDrive behind HTTPS for real deployments. The compose examples use Traefik labels for this.
- Set a stable `SESSION_SECRET`; changing it logs users out.
- Mount your data folders read/write only when users should be able to upload or modify files.
- Keep `DATA_DIR` on persistent storage.
- Install or include `ffmpeg`, `pdftoppm`, and `dcraw_emu` if you want video, PDF, and RAW photo thumbnails.
- Use `NO_SERVER_SIDE_EXECUTION=1` if you want to disable thumbnails, media transcoding, metadata probing, and archive extraction.

## Development

```bash
# Prerequisites: Rust and Node.js 22+
./scripts/dev.sh
```

For a local screenshot/demo environment:

```bash
./scripts/demo.sh
```

## Releases & versioning

Releases are fully automated from [Conventional Commits](https://www.conventionalcommits.org/). Version numbers are never bumped by hand.

| Commit type | Release |
| --- | --- |
| `fix:` / `perf:` | patch (`x.y.Z`) |
| `feat:` | minor (`x.Y.0`) |
| `feat!:`, `fix!:`, … or a `BREAKING CHANGE:` footer | major (`X.0.0`) |
| `docs:`, `test:`, `style:`, `refactor:`, `build:`, `ci:`, `chore:` | no release |

**How it flows:**

1. On every push to `main`, the **Forgejo** pipeline (`.forgejo/workflows/ci.yml`) runs the checks, test, and container build. If they pass, the `release` job runs [`semantic-release`](https://semantic-release.gitbook.io): it works out the next version from the commits, updates `Cargo.toml` + `Cargo.lock` (the authoritative version surfaces), writes `CHANGELOG.md`, commits `chore(release): vX.Y.Z [skip ci]`, and pushes a `vX.Y.Z` tag. Forgejo creates the tag only — no hosted Forgejo release.
2. The commit and tag mirror to GitHub. The **GitHub** workflow (`.github/workflows/publish-container.yml`) triggers on the `v*` tag, builds the container, pushes it to `ghcr.io/kellertobias/nasdrive`, and creates the matching GitHub Release with notes that link the published image.

**Details for maintainers:**

- Release branch: `main`. Tag format: `vX.Y.Z`. Engine: `semantic-release` (pinned in the root `package.json` / `package-lock.json`; this root package is release-only and separate from the app frontend in `web/`).
- Authoritative version files: `Cargo.toml` (`[workspace.package] version`) and `Cargo.lock`. Both crates inherit via `version.workspace = true`. The bump is applied by [`scripts/release/bump-version.sh`](scripts/release/bump-version.sh).
- Required Forgejo secret: `SEMANTIC_RELEASE_TOKEN` — a token with permission to push the release commit to the protected `main` branch and to push tags. The GitHub release uses the automatic `GITHUB_TOKEN` (`contents: write`).
- Preview the next release safely (no tag, no push):

  ```bash
  npm ci
  npx semantic-release --dry-run --no-ci
  ```

## Disclaimer

This repository is fully vibe coded by me to solve the problem of having too little RAM on my NAS to run Nextcloud - and all the other low ressource solutions had a horrible UX. I ran multiple security audits by all major models (even Fable 5.0 in the 2 days it was out) and fixed all problems (of which there weren't many, since I focused on security early on).

Its however still vibe coded, so use at your own risk.

## MIT License

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the “Software”), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED “AS IS”, WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
