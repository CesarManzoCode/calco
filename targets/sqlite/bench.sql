-- Carga de rendimiento de SQLite para bench.py (contrato 8): la misma forma que
-- workload.sql con 10 veces los clientes y productos y 20 veces los pedidos. Todo es
-- determinista: los datos salen de una congruencia lineal, sin azar ni fechas del reloj.
.bail on
.headers on
.mode list
PRAGMA page_size = 4096;
PRAGMA journal_mode = DELETE;

CREATE TABLE customer (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  region TEXT NOT NULL,
  tier INTEGER NOT NULL
);
CREATE TABLE product (
  id INTEGER PRIMARY KEY,
  label TEXT NOT NULL UNIQUE,
  price REAL NOT NULL,
  attrs TEXT NOT NULL
);
CREATE TABLE orders (
  id INTEGER PRIMARY KEY,
  customer_id INTEGER NOT NULL REFERENCES customer(id),
  product_id INTEGER NOT NULL REFERENCES product(id),
  qty INTEGER NOT NULL,
  day INTEGER NOT NULL,
  note TEXT
);

BEGIN;
WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 5000)
INSERT INTO customer(id, name, region, tier)
SELECT i,
       'customer-' || printf('%05d', i),
       CASE (i * 7) % 5 WHEN 0 THEN 'north' WHEN 1 THEN 'south' WHEN 2 THEN 'east'
                        WHEN 3 THEN 'west' ELSE 'center' END,
       1 + (i * 31) % 4
FROM n;

WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 2000)
INSERT INTO product(id, label, price, attrs)
SELECT i,
       'product-' || printf('%04d', i),
       round(((i * 2654435761) % 100000) / 100.0 + 0.5, 2),
       json_object('color', CASE i % 3 WHEN 0 THEN 'red' WHEN 1 THEN 'green' ELSE 'blue' END,
                   'weight', (i * 13) % 97,
                   'tags', json_array('t' || (i % 7), 't' || (i % 11)))
FROM n;

WITH RECURSIVE n(i, x) AS (
  SELECT 1, 12345
  UNION ALL
  SELECT i + 1, (x * 1103515245 + 12345) % 2147483648 FROM n WHERE i < 400000
)
INSERT INTO orders(id, customer_id, product_id, qty, day, note)
SELECT i,
       1 + x % 5000,
       1 + (x / 5000) % 2000,
       1 + (x / 100000) % 9,
       (x / 7) % 365,
       CASE WHEN x % 13 = 0 THEN 'gift' WHEN x % 17 = 0 THEN 'rush' ELSE NULL END
FROM n;
COMMIT;

CREATE INDEX orders_customer ON orders(customer_id);
CREATE INDEX orders_product_day ON orders(product_id, day);
CREATE INDEX customer_region ON customer(region, tier);

SELECT count(*) AS customers FROM customer;
SELECT count(*) AS products FROM product;
SELECT count(*) AS orders, sum(qty) AS units, min(day) AS first_day, max(day) AS last_day
FROM orders;

-- JOIN y agregados por región.
SELECT c.region, count(*) AS n, sum(o.qty) AS units,
       printf('%.2f', sum(o.qty * p.price)) AS revenue,
       printf('%.4f', avg(o.qty)) AS avg_qty
FROM orders o
JOIN customer c ON c.id = o.customer_id
JOIN product p ON p.id = o.product_id
GROUP BY c.region
ORDER BY c.region;

-- GROUP BY con HAVING y orden total.
SELECT o.customer_id, count(*) AS n, sum(o.qty) AS units
FROM orders o
GROUP BY o.customer_id
HAVING count(*) > 95
ORDER BY units DESC, o.customer_id
LIMIT 15;

-- LEFT JOIN: productos sin pedidos en los primeros 30 días.
SELECT count(*) AS idle_products
FROM product p
LEFT JOIN orders o ON o.product_id = p.id AND o.day < 30
WHERE o.id IS NULL;

-- Funciones de ventana.
SELECT customer_id, day, qty, running, rnk
FROM (
  SELECT customer_id, day, qty,
         sum(qty) OVER (PARTITION BY customer_id ORDER BY day, id
                        ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS running,
         rank() OVER (PARTITION BY customer_id ORDER BY qty DESC, id) AS rnk
  FROM orders
  WHERE customer_id IN (7, 77, 177)
)
WHERE rnk <= 4
ORDER BY customer_id, rnk;

SELECT region, tier, n,
       sum(n) OVER (PARTITION BY region ORDER BY tier) AS cumulative,
       lag(n, 1, 0) OVER (PARTITION BY region ORDER BY tier) AS previous,
       ntile(3) OVER (ORDER BY n DESC, region, tier) AS bucket
FROM (SELECT region, tier, count(*) AS n FROM customer GROUP BY region, tier)
ORDER BY region, tier;

-- JSON: extracción, agregado y json_each.
SELECT json_extract(attrs, '$.color') AS color, count(*) AS n,
       sum(json_extract(attrs, '$.weight')) AS weight
FROM product
GROUP BY color
ORDER BY color;

SELECT j.value AS tag, count(*) AS n
FROM product, json_each(product.attrs, '$.tags') AS j
GROUP BY j.value
ORDER BY n DESC, tag
LIMIT 8;

SELECT json_group_array(id) AS heavy
FROM (SELECT id FROM product WHERE json_extract(attrs, '$.weight') > 90 ORDER BY id);

SELECT json_object('region', region, 'customers', count(*),
                   'tiers', json_group_object(CAST(tier AS TEXT), n)) AS summary
FROM (SELECT region, tier, count(*) AS n FROM customer GROUP BY region, tier ORDER BY tier)
GROUP BY region
ORDER BY region;

-- CTE recursiva de consulta (no de inserción).
WITH RECURSIVE fib(k, a, b) AS (
  SELECT 0, 0, 1
  UNION ALL
  SELECT k + 1, b, a + b FROM fib WHERE k < 80
)
SELECT k, a FROM fib WHERE k % 10 = 0;

-- Transacción revertida: no debe dejar rastro.
BEGIN;
DELETE FROM orders WHERE qty > 5;
SELECT count(*) AS during_rollback FROM orders;
ROLLBACK;
SELECT count(*) AS after_rollback FROM orders;

-- Transacción con SAVEPOINT parcialmente revertido.
BEGIN;
UPDATE product SET price = round(price * 1.1, 2) WHERE id % 5 = 0;
SAVEPOINT sp;
DELETE FROM orders WHERE note = 'rush';
ROLLBACK TO sp;
DELETE FROM orders WHERE note = 'gift';
RELEASE sp;
COMMIT;
SELECT count(*) AS orders_left, printf('%.2f', sum(price)) AS price_total
FROM orders JOIN product ON product.id = orders.product_id;

-- Cadenas, agregados de texto y conversiones.
SELECT upper(substr(name, 1, 3)) || '#' || length(name) AS code,
       group_concat(DISTINCT region) AS regions,
       hex(zeroblob(2)) AS z, typeof(1.0 * tier) AS t,
       printf('%x|%e|%10.3f', tier * 255, tier / 3.0, tier * 3.14159) AS fmt
FROM customer
WHERE id IN (1, 2, 3, 4, 5)
GROUP BY id
ORDER BY id;

SELECT p.id, p.label, total
FROM product p
JOIN (SELECT product_id, sum(qty) AS total FROM orders GROUP BY product_id) t
  ON t.product_id = p.id
ORDER BY total DESC, p.id
LIMIT 10;

-- Una vista y un disparador.
CREATE VIEW region_units AS
SELECT c.region, sum(o.qty) AS units
FROM orders o JOIN customer c ON c.id = o.customer_id
GROUP BY c.region;

CREATE TABLE audit (id INTEGER PRIMARY KEY, order_id INTEGER, old_qty INTEGER, new_qty INTEGER);
CREATE TRIGGER orders_qty AFTER UPDATE OF qty ON orders
BEGIN
  INSERT INTO audit(order_id, old_qty, new_qty) VALUES (old.id, old.qty, new.qty);
END;
UPDATE orders SET qty = qty + 1 WHERE day = 100;
SELECT count(*) AS audited, sum(new_qty - old_qty) AS delta FROM audit;
SELECT * FROM region_units ORDER BY region;

DELETE FROM orders WHERE day % 3 = 0;
VACUUM;
PRAGMA integrity_check;
SELECT count(*) AS final_orders FROM orders;
PRAGMA page_count;
