-- Slow order lines lookup for a single cart (SHOP-101).
-- Read only: shows the plan and the row counts involved.
EXPLAIN ANALYZE
SELECT ol.*
FROM order_lines ol
WHERE ol.cart_id = 12345
ORDER BY ol.created_at;

SELECT COUNT(*) AS lines, COUNT(DISTINCT cart_id) AS carts
FROM order_lines;
