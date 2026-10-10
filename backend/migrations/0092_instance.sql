-- This server's identity: one row, made once. A client that knows a server by
-- several addresses (home network, a public name, a VPN) uses it to tell that
-- they all reach the same server.
CREATE TABLE instance (
    singleton  BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    id         UUID NOT NULL DEFAULT gen_random_uuid(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO instance DEFAULT VALUES;
