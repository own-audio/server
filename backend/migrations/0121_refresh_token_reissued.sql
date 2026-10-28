-- A refresh token handed out a second time for a lost response (one retry
-- grace, RFC 9700 §4.14.2): the client sent its token, the server rotated it,
-- the answer never arrived, and the client sent the old token again. Such a
-- token gets no further grace, so a stolen token can be used at most once
-- before the chain is revoked as it always was (`auth::refresh`).
ALTER TABLE refresh_tokens
    ADD COLUMN reissued BOOLEAN NOT NULL DEFAULT false;
