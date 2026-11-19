---
status: done
type: chore
environments: [dev, staging, prod]
notes: Released with 2.14; nothing left
branch: chore/SHOP-88-payment-sdk-5
---
# Upgrade the payment SDK to v5

v4 reaches end of support next quarter. v5 renames the refund API and requires idempotency keys.

## Tasks

- [x] Bump the SDK and fix the compile errors
- [x] Send an idempotency key on every charge and refund
- [x] Smoke test refunds in staging
- [x] Release to production
