# Contributing

Thank you for looking. This is one person's project, built for one family
first and published so others can run it.

## Issues

Welcome, and read. A good bug report has the version (`GET /api/v1/server`),
how you run it (compose, storage kind, library folders or not), what you did
and what happened. For a security problem, see `SECURITY.md` instead.

## Pull requests

Not merged yet. The hosted service at own.audio combines this code with
private code, and that is only possible for contributions whose author has
granted the right to do so: a contributor licence agreement, explained in
[docs/LICENSING.md](docs/LICENSING.md). Until that agreement exists, an
outside pull request cannot be merged, however good it is. Open an issue
instead and describe the change; that is the most useful thing right now.

## Working on it locally

```bash
cp .env.example .env            # set POSTGRES_PASSWORD and SESSION_SECRET
docker compose up -d postgres
cd backend && cargo clippy --all-targets -- -D warnings && cargo test
cd frontend && npm ci && npm run i18n && npm run build
docker compose up -d --build    # the whole stack from this checkout
python3 conformance/run.py --base-url http://localhost:8080 \
    --admin-email <your admin> --admin-password '<password>'
```

The API is a contract with clients that are released separately: changes to
`/api/v1` are additive only, with a revision bump
([docs/API_COMPATIBILITY.md](docs/API_COMPATIBILITY.md)). Every change goes
into `CHANGELOG.md`.
