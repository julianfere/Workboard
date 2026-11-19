-- SHOP-104: gift cards and their redemption ledger.
CREATE TABLE gift_cards (
    id             BIGSERIAL PRIMARY KEY,
    code           CHAR(16)      NOT NULL UNIQUE,
    initial_amount NUMERIC(10,2) NOT NULL CHECK (initial_amount > 0),
    currency       CHAR(3)       NOT NULL DEFAULT 'USD',
    created_at     TIMESTAMPTZ   NOT NULL DEFAULT now(),
    expires_at     TIMESTAMPTZ
);

CREATE TABLE gift_card_redemptions (
    id           BIGSERIAL PRIMARY KEY,
    gift_card_id BIGINT        NOT NULL REFERENCES gift_cards(id),
    order_id     BIGINT        NOT NULL,
    amount       NUMERIC(10,2) NOT NULL CHECK (amount > 0),
    created_at   TIMESTAMPTZ   NOT NULL DEFAULT now()
);
