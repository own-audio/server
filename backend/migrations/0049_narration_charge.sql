-- Migration 0049: AI narration is a real charge against family credit.
--
-- Generation jobs computed `charged_price_cents` and then did nothing with
-- it: the price was shown at the quote, confirmed by the user, and never
-- appeared in the family's spending. Storage was the only thing that ever
-- moved the balance.
--
-- Idempotency mirrors `topup`'s: the generation job id goes in `external_ref`
-- with a unique partial index, so a retried or double-dispatched assembly
-- job hits ON CONFLICT DO NOTHING rather than charging twice.
ALTER TABLE credit_ledger DROP CONSTRAINT IF EXISTS credit_ledger_entry_type_check;
ALTER TABLE credit_ledger
    ADD CONSTRAINT credit_ledger_entry_type_check
    CHECK (entry_type IN ('welcome_grant', 'grant', 'storage_charge',
                          'adjustment', 'topup', 'narration_charge'));

-- Charges are zero-or-negative and always carry the job that caused them, so
-- a ledger row can always be traced back to the book it paid for.
ALTER TABLE credit_ledger DROP CONSTRAINT IF EXISTS credit_ledger_narration_check;
ALTER TABLE credit_ledger
    ADD CONSTRAINT credit_ledger_narration_check
    CHECK (entry_type <> 'narration_charge'
           OR (amount_micro <= 0 AND external_ref IS NOT NULL));

CREATE UNIQUE INDEX IF NOT EXISTS credit_ledger_narration_uq
    ON credit_ledger (external_ref)
    WHERE entry_type = 'narration_charge';
