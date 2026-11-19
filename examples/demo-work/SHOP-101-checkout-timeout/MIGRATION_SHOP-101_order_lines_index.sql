-- SHOP-101: index for the cart lookup used by checkout.
CREATE INDEX CONCURRENTLY IF NOT EXISTS ix_order_lines_cart_created
    ON order_lines (cart_id, created_at);
