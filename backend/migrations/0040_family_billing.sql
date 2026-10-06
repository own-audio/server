-- Migration 0040: family credit ledger + daily storage billing
-- ────────────────────────────────────────────────────────────
-- Design rules:
--   • Money is BIGINT micro-USD (1_000_000 = $1). A small library costs
--     ~1.3¢/day at $0.05/GB/month; cents would round that to zero.
--   • Payments don't exist yet (no Stripe): every balance is play money —
--     a welcome grant minus daily storage charges. The ledger shape is
--     already what a real payment integration would append to.
--   • Idempotency lives in the schema, not in worker logic: at most one
--     storage charge per family per UTC day, at most one welcome grant per
--     user ever. Two worker containers racing the same sweep both hit
--     ON CONFLICT DO NOTHING instead of double-charging.

CREATE TABLE credit_ledger (
    id            UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    family_id     UUID        NOT NULL REFERENCES families (id) ON DELETE CASCADE,
    entry_type    TEXT        NOT NULL CHECK (entry_type IN
                              ('welcome_grant', 'grant', 'storage_charge', 'adjustment')),
    -- Signed: grants positive, charges zero-or-negative.
    amount_micro  BIGINT      NOT NULL,
    -- Storage snapshot (bytes) the charge was computed from; charges only.
    storage_bytes BIGINT,
    -- The UTC day a storage charge covers; charges only.
    charge_date   DATE,
    -- The user whose once-ever welcome grant this is; welcome grants only.
    user_id       UUID        REFERENCES users (id) ON DELETE SET NULL,
    note          TEXT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),

    CHECK (entry_type <> 'storage_charge'
           OR (charge_date IS NOT NULL AND storage_bytes IS NOT NULL AND amount_micro <= 0))
);

-- One storage charge per family per day — the billing sweep's idempotency.
CREATE UNIQUE INDEX credit_ledger_daily_charge_uq
    ON credit_ledger (family_id, charge_date)
    WHERE entry_type = 'storage_charge';

-- One welcome grant per user, ever. Without this, leave-family → fresh
-- personal family → new grant would mint credit in a loop.
CREATE UNIQUE INDEX credit_ledger_welcome_once_uq
    ON credit_ledger (user_id)
    WHERE entry_type = 'welcome_grant';

CREATE INDEX credit_ledger_family_idx ON credit_ledger (family_id, created_at DESC);

-- The storage sum filters media_objects by the family key prefix
-- (`f/{family_id}/…`); text_pattern_ops is what lets LIKE 'f/…%' use it.
CREATE INDEX media_objects_key_prefix_idx
    ON media_objects (object_key text_pattern_ops);

-- ── Backfill ────────────────────────────────────────────────────────────────
-- Every existing user gets their $5 welcome credit, deposited into whatever
-- family they belong to right now. The amount mirrors the public pricing
-- page's "$5 free" figure (audio2-www Pricing.astro — hand-duplicated there).
INSERT INTO credit_ledger (family_id, entry_type, amount_micro, user_id, note)
SELECT fm.family_id, 'welcome_grant', 5000000, fm.user_id, 'Welcome credit'
FROM family_members fm;
