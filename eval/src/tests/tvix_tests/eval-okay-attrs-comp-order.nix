with import ./lib.nix;
let
  makeSet =
    name: value:
    (builtins.listToAttrs (
      map
        (n: {
          name = "${name}${builtins.toString n}";
          value = value;
        })
        (range 1 100)
    ));

  # Comparison happens in lexical order, depth first.
  # So it should return with false, before encountering any of the error fields
  early1 = {
    a = {
      a = true;
      b = throw "error";
    };
  } // makeSet "error" (throw "err");

  early2 = {
    a = {
      a = false;
      b = throw "error";
    };
  } // makeSet "error" (throw "err");

  # Hits error first
  error1 = {
    a = throw "error";
  } // makeSet "foo" true;

  error2 = {
    a = throw "error";
  } // makeSet "foo" false;

in
[
  (early1 == early2)
  (builtins.tryEval (error1 == error2)).success
  # Compares name first, so doesn't hit the error
  ({ foo = throw "err"; } != { bar = throw "err"; })
]
