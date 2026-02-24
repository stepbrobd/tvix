# Pointer equality behavior specific to Lix >= 2.94, see
# <https://lix.systems/blog/2025-11-18-lix-2.94-release/#language>.
# All behavior portable between C++ Nix, Lix and Tvix is covered
# by eval-okay-value-pointer-equality.nix.
let
  # Some incomparable values
  f = MC: "Boing";
  t = [ (throw "is a little blue man") ];
  a = { "with" = abort "headphones and a big smile."; };
  b = builtins.getAttr "foo";

  # Aliases
  f' = f;
  t' = t;
  a' = a;

  id = x: x;

  # Wrappers
  s.f = f;
  l = [ f ];

  # Aliases
in
[
  # Only functions and builtins can be compared directly
  (f == f)
  (f == f')
  (b == b)
  (s.f == s.f)
  (s.f == f)
  (builtins.head l == builtins.head l)
  (f == builtins.head l)
  (id f == id f)
  (builtins.add == builtins.add)
  (with s; f == f)
  (with builtins; add == add)

  # Lix also compares structures that would fail eval when forced
  (t == t)
  (t' == t)
  (t == { inherit t; }.t)
  (a == a)
  (a' == a)
  (builtins.head [ a ] == a)
  (id a == a)
  (id t == id t)

  ({ inherit (s) f; } == { inherit (s) f; })
]
