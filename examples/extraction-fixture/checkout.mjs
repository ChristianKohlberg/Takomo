// Small, dependency-free source sample for scoped specification extraction.
export function quote(items, { member = false } = {}) {
  if (!Array.isArray(items) || items.length === 0) throw new Error('At least one item is required.');
  let subtotal = 0;
  for (const item of items) {
    if (!Number.isInteger(item.quantity) || item.quantity < 1 || item.quantity > 20) throw new Error('Quantity must be between 1 and 20.');
    if (!Number.isInteger(item.unitPriceCents) || item.unitPriceCents < 0) throw new Error('Prices must be nonnegative integer cents.');
    subtotal += item.quantity * item.unitPriceCents;
  }
  const discount = member ? Math.floor(subtotal / 10) : 0;
  const shipping = subtotal - discount >= 5000 ? 0 : 500;
  return { subtotal, discount, shipping, total: subtotal - discount + shipping };
}

export function cancel(order, actor) {
  if (order.customerId !== actor.id && actor.role !== 'admin') throw new Error('Only the customer or an administrator may cancel.');
  if (order.status !== 'pending') throw new Error('Only pending orders can be cancelled.');
  return { ...order, status: 'cancelled' };
}
