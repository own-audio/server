-- Wrong passwords in a row, per submitted email (security hardening plan
-- §5.1). Keyed by what was typed, lower-cased, not by a user: an email that
-- has no account is counted the same way, so the lock cannot be used to
-- find out which emails exist. Rows are cleared on a successful sign-in.
CREATE TABLE login_failures (
    email_lc        TEXT        PRIMARY KEY,
    failures        INTEGER     NOT NULL DEFAULT 0,
    last_failure_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    locked_until    TIMESTAMPTZ
);
