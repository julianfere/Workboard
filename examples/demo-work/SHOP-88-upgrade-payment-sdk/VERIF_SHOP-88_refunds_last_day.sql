-- SHOP-88: refunds of the last 24 h and whether they carry an idempotency key.
SELECT status, COUNT(*) AS refunds, COUNT(idempotency_key) AS with_key
FROM payment_refunds
WHERE created_at > now() - INTERVAL '1 day'
GROUP BY status;
