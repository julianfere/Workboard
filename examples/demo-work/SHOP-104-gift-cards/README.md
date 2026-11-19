---
status: planned
type: feature
notes: Plan ready; start with the schema and the balance endpoint
branch: feature/SHOP-104-gift-cards
due: 2026-12-11
period: 2026-12
hours: 24
---
# Gift cards

## Description

Customers can buy a gift card with a fixed amount and redeem it, fully or partially, at checkout.

## Acceptance criteria

- A gift card has a unique 16-character code and a balance.
- Redeeming more than the balance is rejected with a clear message.
- Partial redemptions keep the remaining balance.

## Plan

Add a `gift_cards` table and a `gift_card_redemptions` ledger (see `MIGRATION_SHOP-104_gift_cards.sql`).
The balance is always computed from the ledger, never stored twice. Checkout calls a new
`POST /gift-cards/{code}/redeem` endpoint inside the order transaction, so a failed order rolls the
redemption back. Details and open questions in `PLAN_gift-cards.md`.

## Tasks

- [ ] Create the tables (migration)
- [ ] Balance endpoint `GET /gift-cards/{code}`
- [ ] Redeem endpoint inside the order transaction
- [ ] Checkout UI: "Use a gift card" field
- [ ] Tests for partial and over-balance redemptions
