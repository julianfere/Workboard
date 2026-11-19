# Gift cards: plan notes

## Data model

- `gift_cards(id, code, initial_amount, currency, created_at, expires_at)`
- `gift_card_redemptions(id, gift_card_id, order_id, amount, created_at)`

Balance = `initial_amount - sum(redemptions.amount)`.

## Open questions

- Do gift cards expire? Product says "probably after 2 years"; the column is nullable for now.
- Multiple currencies are out of scope for the first version.
