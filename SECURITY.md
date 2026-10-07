# Security

Please report security problems privately, not in a public issue: e-mail
**security@own.audio**. Include what you found,
how to reproduce it and which version (`GET /api/v1/server` → `version`).

You will get an answer within a week. A fix goes into the next release, with
credit in `CHANGELOG.md` if you want it. This is one person's project with no
bounty, but every report is read and taken seriously.

## Supported versions

The newest release only. Until 1.0 the server is pre-release software, and
fixes are not backported to older alphas.

## What is in scope

The server in this repository and its web console: authentication and
sessions, family and sharing boundaries (one member reading another's
private items is a security bug), signed media links, file paths in library
folders and file sync, and the container image. The hosted service at
own.audio runs the same code, so the same report covers both.

Out of scope: denial of service by sheer volume, and problems that need an
admin account to begin with.

## For people running it

- Set `SESSION_SECRET` to a random value (`openssl rand -hex 32`). The server
  refuses to start with an empty, placeholder or short one: it signs every
  sign-in and media link.
- Keep open registration off (the default) and add people by invite.
- Put it behind HTTPS before it is reachable from the internet (a reverse
  proxy or a Cloudflare Tunnel, `INSTALL.md`), and set
  `SERVER__RATE_LIMIT__TRUST_PROXY_HEADERS=true` when a proxy is in front.
- The server runs as an unprivileged user and only reads your library
  folders; mount them read-only (`:ro`).
- Keep `backups/` as private as the server: it holds the whole database.
