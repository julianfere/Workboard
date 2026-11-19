---
status: testing
type: bug
environments: [dev, staging]
notes: In staging; QA checks the link with the new mail provider
branch: fix/SHOP-97-reset-link
due: 2026-11-24
---
# Password reset email has an expired link

The reset link in the email expires after 5 minutes instead of 60 because the token TTL is read in
seconds but configured in minutes.

## Tasks

- [x] Fix the unit of `RESET_TOKEN_TTL`
- [x] Add a test for the expiry
- [x] Deploy to staging
- [ ] QA: request a reset and use the link after 30 minutes
