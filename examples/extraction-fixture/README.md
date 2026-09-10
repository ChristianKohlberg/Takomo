# Specification extraction sample

Select only `examples/extraction-fixture/checkout.mjs` for the smallest run, or
`examples/extraction-fixture` for this complete sample. No dependencies, network,
payment provider or database are needed. The importer should read this code, not
execute it. Limit the first draft to three sections.

Review the resulting Document and Map against these deliberately concrete rules:

- An order must contain items; quantities are integer values from 1 through 20.
- Prices use nonnegative integer cents.
- Members receive a 10% discount, rounded down to whole cents.
- Shipping is free when the discounted subtotal reaches 5,000 cents; otherwise
  it costs 500 cents.
- A customer can cancel their own pending order. An administrator can cancel
  any pending order. Other order states cannot be cancelled.

The sample does not charge payments, persist orders or authenticate users.
Those are coverage gaps, not requirements the importer should invent.
