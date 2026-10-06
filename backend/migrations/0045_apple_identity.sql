-- Migration 0045: allow 'apple' as an auth_identities provider
-- ───────────────────────────────────────────────────────────
-- See docs/sso-payments-plan.md. Sign in with Apple joins Google/Microsoft
-- as a supported identity provider; the (provider, provider_subject)
-- uniqueness and one-identity-per-provider-per-user constraints from
-- migration 0002 already cover it without changes.

ALTER TABLE auth_identities DROP CONSTRAINT auth_identities_provider_check;
ALTER TABLE auth_identities ADD CONSTRAINT auth_identities_provider_check
    CHECK (provider IN ('local', 'google', 'microsoft', 'apple'));
