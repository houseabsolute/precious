# Upgrading from 0.3.0 to 0.4.0

Some of the command configuration has changed dramatically in this release. The old `run_mode` and
`chdir` config keys have been replaced by new options, `invoke`, `working_dir`, and `path_args`.
**Support for the old keys was removed in 0.10.0. As of 0.12.0, a config that still contains them
will fail to load with an "unknown field" error, rather than being silently ignored.**

Here is what the new config looks like for all possible combinations of the `run_mode` and `chdir`
keys.

---

```toml
run_mode = "files"
chdir = false
```

This was the default, and the equivalent defaults produce the same result:

```toml
invoke = "per-file"
working_dir = "root"
path_args = "file"
```

---

```toml
run_mode = "files"
chdir = true
```

```toml
invoke = "per-file"
working_dir = "dir"
path_args = "file"
```

---

```toml
run_mode = "dirs"
chdir = false
```

```toml
invoke = "per-dir"
working_dir = "root"
path_args = "dir"
```

---

```toml
run_mode = "dirs"
chdir = true
```

```toml
invoke = "per-dir"
working_dir = "dir"
path_args = "none"
```

---

```toml
run_mode = "root"
chdir = false
```

```toml
invoke = "once"
working_dir = "root"
path_args = "dot"
```

---

```toml
run_mode = "root"
chdir = true
```

```toml
invoke = "once"
working_dir = "root"
path_args = "none"
```

---

But note that the new config options allow for many other possibilities that couldn't be expressed
with the old options.
