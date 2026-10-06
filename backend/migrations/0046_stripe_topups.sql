-- Migration 0046: Stripe Checkout credit top-ups
-- ────────────────────────────────────────────────
-- See docs/sso-payments-plan.md (Phase B). A 'topup' ledger entry is real
-- money in, via a Stripe Checkout session; `external_ref` holds the
-- checkout-session id and doubles as the webhook's idempotency key so a
-- retried/duplicated webhook delivery can never double-credit a family.

ALTER TABLE credit_ledger DROP CONSTRAINT credit_ledger_entry_type_check;
ALTER TABLE credit_ledger ADD CONSTRAINT credit_ledger_entry_type_check
    CHECK (entry_type IN ('welcome_grant', 'grant', 'storage_charge',
                          'adjustment', 'topup'));

ALTER TABLE credit_ledger ADD COLUMN external_ref TEXT;

ALTER TABLE credit_ledger ADD CONSTRAINT credit_ledger_topup_check
    CHECK (entry_type <> 'topup' OR (amount_micro > 0 AND external_ref IS NOT NULL));

-- One ledger row per Stripe checkout session, ever — the primary defence
-- against a replayed or duplicated webhook delivery double-crediting.
CREATE UNIQUE INDEX credit_ledger_topup_ref_uq
    ON credit_ledger (external_ref) WHERE entry_type = 'topup';

-- Processed webhook events. A secondary idempotency guard (Stripe can and
-- does redeliver the same event id) and a debugging trail independent of
-- the ledger.
CREATE TABLE stripe_events (
    id          TEXT        PRIMARY KEY,
    event_type  TEXT        NOT NULL,
    received_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
