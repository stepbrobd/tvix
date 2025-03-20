# TODO

This contains a rough collection of ideas on the TODO list, trying to keep track
of it somewhere.

Of course, there's no guarantee these things will get addressed, but it helps
dumping the backlog somewhere.

## Cleanups
### Correctness > Performance
A lot of the Nix behaviour isn't well documented out, and before going too deep
into performance optimizations, we need to ensure we properly grasped all hidden
features. This is to avoid doing a lot of "overall architecture perf-related
work" and increased code complexity based on a mental model that might get
disproved later on, as we work towards correctness.

We do this by evaluating more and more parts of the official Nix test suite, as
well as our own Tvix test suite, and compare it with Nix' output.

Additionally, we evaluate attributes from nixpkgs, compare calculated output
paths (to determine equivalence of evaluated A-Terms) and fix differences as we
encounter them.

This currently is a very manual and time-consuming process, both in terms of
setup, as well as spotting the source of the differences (and "compensating" for
the resulting diff noise on resulting mismtaches).

 - We could use some better tooling that periodically evaluates nixpkgs, and
   compares the output paths with the ones produced by Nix
 - We could use some better tooling that can spot the (real) differences between
   two (graphs of) derivations, while removing all resulting noise from the diff
in resulting store paths.


### Performance
Even while keeping in mind some of the above caveats, there's some obvious
low-langing fruits that could have a good impact on performance, with somewhat
limited risk of becoming obsolete in case of behaviorial changes due to
correctness:

 - String Contexts currently do a lot of indirections (edef)
   (NixString -> NixStringInner -> HashSet[element] -> NixContextElement -> String -> data)
   to get to the actual data. We should improve this. There's various ideas, one
   of it is globally interning all Nix context elements, and only keeping
   indices into that. We might need to have different representations for small
   amount of context elements or larger ones, and need tooling to reason about
   the amount of contexts we have.

## Fixes towards correctness
 - `rnix` only supports string source files, but `NixString` uses bytes (and Nix
   source code might be no valid UTF-8).

## Documentation
 - Update the Architecture diagram to model the current state of things.
   There's no gRPC between Coordinator and Evaluator.

## Features

### Fetchers
Some more fetcher-related builtins need work:
 - `fetchGit`
 - `fetchTree` (hairy, seems there's no proper spec and the URL syntax seems
   subject to change/underdocumented)
