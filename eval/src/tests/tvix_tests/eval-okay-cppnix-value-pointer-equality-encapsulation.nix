# For an explanation of this behavior see //tvix/docs/value-pointer-equality.md
#
# This test documents that for a value to be pointer equal in C++ Nix, it needs
# to be wrapped in another value (a list or attrset). This is no longer the case
# with Lix >= 2.94 (as of 2026-05-17).
let
  # With encapsulation, we can also compare thunks that `throw` or `abort`
  # thanks to laziness. Functions are the only values that can only be equal
  # by pointer and do not stop evaluation, so that's what we use here.
  f = MC: "Boing";
  # Alias
  f' = f;
in
[
  (f == f)
  (f == f')
]
