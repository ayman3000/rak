Add discount codes to the shop package.

- `Cart.apply_code(code)` takes a discount code and remembers it. Only one code is active at a time: a later valid code replaces the earlier one. Codes are not case-sensitive. An unknown code raises ValueError and leaves the active code as it was.
- The codes: SAVE10 takes 10% off the subtotal. FLAT5 takes 5.00 off the subtotal, but the subtotal never goes below 0. FREESHIP is accepted and does not change the total.
- The discount comes off the subtotal before tax is added. `Cart.total()` returns the discounted total with tax, rounded to 2 decimals.
- `pricing.total(lines, code=None)` takes the code as well and gives the same result as the cart. With no code it works as it does now. An unknown code raises ValueError there too.
- Adding an item whose name is already in the cart raises that line's qty instead of adding a second line. The price stays the one from the first add.
