-- Migration 0041: family credit alert preferences
-- ────────────────────────────────────────────────
-- See docs/billing-settings-widgets-plan.md decisions 2-3.
--
-- One row per family, created on first PUT (upsert pattern, same as
-- user_settings). Absent row means "no alerts configured" — the correct
-- default for every existing family, so no backfill.
--
-- balance_alert_active / days_alert_active are an edge-trigger latch, not a
-- log: the daily sweep only queues a notification on a below-threshold
-- transition (false -> true), and clears the latch once the family climbs
-- back above the threshold (without notifying on the way back up). Without
-- this, a family sitting under the threshold would get a notification every
-- single day forever.

CREATE TABLE credit_alerts (
    family_id            UUID        PRIMARY KEY REFERENCES families (id) ON DELETE CASCADE,
    -- NULL means that rule is off.
    min_balance_micro    BIGINT      CHECK (min_balance_micro IS NULL OR min_balance_micro > 0),
    min_days_remaining   INTEGER     CHECK (min_days_remaining IS NULL OR min_days_remaining > 0),
    balance_alert_active BOOLEAN     NOT NULL DEFAULT false,
    days_alert_active    BOOLEAN     NOT NULL DEFAULT false,
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT now()
);
