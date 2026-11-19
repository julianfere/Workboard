---
status: in-progress
type: bug
environments: [dev]
notes: Index added in dev; waiting for a staging run to confirm the timeout is gone
branch: fix/SHOP-101-checkout-timeout
due: 2026-11-28
hours: 6
---
# Checkout times out for carts with many items

Customers with more than 40 items in the cart get a gateway timeout when they press **Place order**.
The order query scans the whole `order_lines` table.

## Tasks

- [x] Reproduce with a 50-item cart in dev
- [x] Find the slow query (see `DIAG_SHOP-101_slow_order_lines.sql`)
- [x] Add the missing index (`MIGRATION_SHOP-101_order_lines_index.sql`)
- [ ] Run the migration in staging
- [ ] Load-test checkout with 100 items

## How to deploy

1. Run `MIGRATION_SHOP-101_order_lines_index.sql` (online index build, safe during business hours).
2. Deploy the API from `fix/SHOP-101-checkout-timeout`.
