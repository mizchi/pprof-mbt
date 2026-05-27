# pprof-stack-formats

Convert pprof CPU profiles to:

- folded stacks (`root;child;leaf value`)
- Speedscope JSON

and convert Speedscope sampled profiles back to gzip-compressed pprof.

Used by `moon-pprof pprof2folded`, `moon-pprof pprof2speedscope`, and
`moon-pprof speedscope2pprof`.
