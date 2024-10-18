## Engineering code of conduct 🤝

Here we describe certain regularities which help to streamline collaboration.

### Linter

Use `prettier` to check formatting of all other files in the repository:

```bash
npm i
npm run prettier:check
npm run prettier:fix
```

Use `clippy` to lint your rust code:

```bash
cargo clippy
```

### Documentation

We publish dev docs on every push to `GitBook`.
Check them out at https://distributed-lab.github.io/bpcon/.

### Releasing

We use `Semantic Release`. You need is to make PR titles to conform with `conventional commits`
and everything else will be done automatically. Additionally, please make sure that you are
actualizing library version in `Cargo.toml`.

No need to push tags, create releases yourself or merge release branches.
