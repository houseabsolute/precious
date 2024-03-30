# Invocation Examples

The following examples illustrate how `precious` executes a command with different invocation
options.

For these examples we will assume that the command is configured to execute for any file ending in
`.go`. The executable is `some-linter` and only takes paths as argument. The file tree looks like
this:

```
example
├── app.go
├── main.go
├── pkg1
│  ├── pkg1.go
├── pkg2
│  ├── pkg2.go
│  ├── pkg2_test.go
│  └── subpkg
│     └── subpkg.go
└── precious.toml
```

---

- **Runs once per file**
- **Working directory is the project root**
- **Relative file path as the argument**

This is the default configuration.

```toml
[commands.some-linter]
invoke = "per-file"
working-dir = "root"
path-args = "file"
```

```
some-linter app.go
some-linter main.go
some-linter pkg1/pkg1.go
some-linter pkg2/pkg2.go
some-linter pkg2/pkg2_test.go
some-linter pkg2/subpkg/subpkg.go
```

---

- **Runs once per file**
- **From the root**
- **Absolute file path as the argument**

```toml
[commands.some-linter]
invoke = "per-file"
working-dir = "root"
path-args = "absolute-file"
```

```
some-linter /example/app.go
some-linter /example/main.go
some-linter /example/pkg1/pkg1.go
some-linter /example/pkg2/pkg2.go
some-linter /example/pkg2/pkg2_test.go
some-linter /example/pkg2/subpkg/subpkg.go
```

---

- **Runs once per file**
- **Working directory changes per-directory**
- **Relative file path as the argument**

```toml
[commands.some-linter]
invoke = "per-file"
working-dir = "dir"
path-args = "file"
```

```
some-linter app.go
some-linter main.go
cd /example/pkg1
some-linter pkg1.go
cd /example/pkg2
some-linter pkg2.go
some-linter pkg2_test.go
cd /example/pkg2/subpkg
some-linter subpkg.go
```

---

- **Runs once per file**
- **Working directory changes per-directory**
- **Absolute file path as the argument**

```toml
[commands.some-linter]
invoke = "per-file"
working-dir = "dir"
path-args = "absolute-file"
```

```
some-linter /example/app.go
some-linter /example/main.go
cd /example/pkg1
some-linter /example/pkg1/pkg1.go
cd /example/pkg2
some-linter /example/pkg2/pkg2.go
some-linter /example/pkg2/pkg2_test.go
cd /example/pkg2/subpkg
some-linter /example/pkg2/subpkg/subpkg.go
```

It's odd to combine `working-dir = "dir"` with `path-args = "absolute-file"`, but it will work.

---

- **Runs once per file**
- **Working directory is the given subdirectory**
- **Relative file path as the argument**

```toml
[commands.some-linter]
invoke = "per-file"
working-dir.chdir-to = "pkg1"
path-args = "file"
```

```
cd /example/pkg1
some-linter ../app.go
some-linter ../main.go
some-linter ../pkg2/pkg2.go
some-linter ../pkg2/pkg2_test.go
some-linter ../pkg2/subpkg/subpkg.go
some-linter pkg1.go
```

---

- **Runs once per file**
- **Working directory is the given subdirectory**
- **Absolute file path as the argument**

```toml
[commands.some-linter]
invoke = "per-file"
working-dir.chdir-to = "pkg1"
path-args = "absolute-file"
```

```
cd /example/pkg1
some-linter /example/app.go
some-linter /example/main.go
some-linter /example/pkg1/pkg1.go
some-linter /example/pkg2/pkg2.go
some-linter /example/pkg2/pkg2_test.go
some-linter /example/pkg2/subpkg/subpkg.go
```

---

- **Runs once per directory**
- **Working directory is the root**
- **Relative directory path as pargument**

```toml
[commands.some-linter]
invoke = "per-dir"
working-dir = "root"
path-args = "dir"
```

```
some-linter .
some-linter pkg1
some-linter pkg2
some-linter pkg2/subpkg
```

---

- **Runs once per directory**
- **Working directory is the root**
- **Absolute directory path as pargument**

```toml
[commands.some-linter]
invoke = "per-dir"
working-dir = "root"
path-args = "absolute-dir"
```

```
some-linter /example
some-linter /example/pkg1
some-linter /example/pkg2
some-linter /example/pkg2/subpkg
```

---

- **Runs once per directory**
- **Working directory is the root**
- **Relative file paths as arguments**

```toml
[commands.some-linter]
invoke = "per-dir"
working-dir = "root"
path-args = "file"
```

```
some-linter app.go main.go
some-linter pkg1/pkg1.go
some-linter pkg2/pkg2.go pkg2/pkg2_test.go
some-linter pkg2/subpkg/subpkg.go
```

---

- **Runs once per directory**
- **Working directory is the root**
- **Absolute file paths as arguments**

```toml
[commands.some-linter]
invoke = "per-dir"
working-dir = "root"
path-args = "absolute-file"
```

```
some-linter /example/app.go /example/main.go
some-linter /example/pkg1/pkg1.go
some-linter /example/pkg2/pkg2.go /example/pkg2/pkg2_test.go
some-linter /example/pkg2/subpkg/subpkg.go
```

---

- **Runs once per directory**
- **Working directory is each directory in turn**
- **Dot (`.`) as the path argument**

```toml
[commands.some-linter]
invoke = "per-dir"
working-dir = "dir"
path-args = "dot"
```

```
some-linter .
cd /example/pkg1
some-linter .
cd /example/pkg2
some-linter .
cd /example/pkg2/subpkg
some-linter .
```

---

- **Runs once per directory**
- **Working directory is each directory in turn**
- **No path argument**

```toml
[commands.some-linter]
invoke = "per-dir"
working-dir = "dir"
path-args = "none"
```

```
some-linter
cd /example/pkg1
some-linter
cd /example/pkg2
some-linter
cd /example/pkg2/subpkg
some-linter
```

---

- **Runs once per directory**
- **Working directory is the given subdirectory**
- **Relative file paths as the argument**

```toml
[commands.some-linter]
invoke = "per-dir"
working-dir.chdir-to = "pkg1"
path-args = "file"
```

```
cd /example/pkg1
some-linter ../app.go ../main.go
some-linter ../pkg2/pkg2.go ../pkg2/pkg2_test.go
some-linter ../pkg2/subpkg/subpkg.go
some-linter pkg1.go
```

---

- **Runs once per directory**
- **Working directory is the given subdirectory**
- **Absolute file paths as the argument**

```toml
[commands.some-linter]
invoke = "per-dir"
working-dir.chdir-to = "pkg1"
path-args = "absolute-file"
```

```
cd /example/pkg1
some-linter /example/app.go /example/main.go
some-linter /example/pkg1/pkg1.go
some-linter /example/pkg2/pkg2.go /example/pkg2/pkg2_test.go
some-linter /example/pkg2/subpkg/subpkg.go
```

---

- **Runs once per directory**
- **Working directory is the given subdirectory**
- **Relative directory path as the argument**

```toml
[commands.some-linter]
invoke = "per-dir"
working-dir.chdir-to = "pkg1"
path-args = "dir"
```

```
cd /example/pkg1
some-linter .
some-linter ..
some-linter ../pkg2
some-linter ../pkg2/subpkg
```

---

- **Runs once per directory**
- **Working directory is the given subdirectory**
- **Absolute directory path as the argument**

```toml
[commands.some-linter]
invoke = "per-dir"
working-dir.chdir-to = "pkg1"
path-args = "absolute-dir"
```

```
cd /example/pkg1
some-linter /example
some-linter /example/pkg1
some-linter /example/pkg2
some-linter /example/pkg2/subpkg
```

---

- **Runs once for all files**
- **Working directory is the root**
- **Relative file paths as arguments**

```toml
[commands.some-linter]
invoke = "once"
working-dir = "root"
path-args = "file"
```

```
some-linter \
    app.go \
    main.go \
    pkg1/pkg1.go \
    pkg2/pkg2.go \
    pkg2/pkg2_test.go \
    pkg2/subpkg/subpkg.go
```

---

- **Runs once for all files**
- **Working directory is the root**
- **Absolute file paths as arguments**

```toml
[commands.some-linter]
invoke = "once"
working-dir = "root"
path-args = "absolute-file"
```

```
some-linter \
    /example/app.go \
    /example/main.go \
    /example/pkg1/pkg1.go \
    /example/pkg2/pkg2.go \
    /example/pkg2/pkg2_test.go \
    /example/pkg2/subpkg/subpkg.go
```

---

- **Runs once for all directories**
- **Working directory is the root**
- **Relative directory paths as arguments**

```toml
[commands.some-linter]
invoke = "once"
working-dir = "root"
path-args = "dir"
```

```
some-linter . pkg1 pkg2 pkg2/subpkg
```

---

- **Runs once for all directories**
- **Working directory is the root**
- **Absolute directory paths as arguments**

```toml
[commands.some-linter]
invoke = "once"
working-dir = "root"
path-args = "absolute-dir"
```

```
some-linter \
    /example \
    /example/pkg1 \
    /example/pkg2 \
    /example/pkg2/subpkg
```

---

- **Runs once for all paths**
- **Working directory is the root**
- **Dot (`.`) as the path argument**

```toml
[commands.some-linter]
invoke = "once"
working-dir = "root"
path-args = "dot"
```

```
some-linter .
```

---

- **Runs once for all paths**
- **Working directory is the root**
- **No path argument**

```toml
[commands.some-linter]
invoke = "once"
working-dir = "root"
path-args = "none"
```

```
some-linter
```

---

- **Runs once**
- **Working directory is the given subdirectory**
- **Relative file paths as the arguments**

```toml
[commands.some-linter]
invoke = "once"
working-dir.chdir-to = "pkg1"
path-args = "file"
```

```
cd /example/pkg1
some-linter ../app.go ../main.go pkg1.go ../pkg2/pkg2.go ../pkg2/pkg2_test.go ../pkg2/subpkg/subpkg.go
```

---

- **Runs once**
- **Working directory is the given subdirectory**
- **Absolute file paths as the arguments**

```toml
[commands.some-linter]
invoke = "once"
working-dir.chdir-to = "pkg1"
path-args = "absolute-file"
```

```
cd /example/pkg1
some-linter \
    /example/app.go \
    /example/main.go \
    /example/pkg1/pkg1.go \
    /example/pkg2/pkg2.go \
    /example/pkg2/pkg2_test.go \
    /example/pkg2/subpkg/subpkg.go
```

---

- **Runs once**
- **Working directory is the given subdirectory**
- **Relative directory paths as the arguments**

```toml
[commands.some-linter]
invoke = "once"
working-dir.chdir-to = "pkg1"
path-args = "dir"
```

```
cd /example/pkg1
some-linter .. . ../pkg2 ../pkg2/subpkg
```

---

- **Runs once**
- **Working directory is the given subdirectory**
- **Absolute directory paths as the arguments**

```toml
[commands.some-linter]
invoke = "once"
working-dir.chdir-to = "pkg1"
path-args = "absolute-dir"
```

```
cd /example/pkg1
some-linter /example /example/pkg1 /example/pkg2 /example/pkg2/subpkg
```

---

- **Runs once**
- **Working directory is the given subdirectory**
- **Dot (`.`) as the path argument**

```toml
[commands.some-linter]
invoke = "once"
working-dir.chdir-to = "pkg1"
path-args = "dot"
```

```
cd /example/pkg1
some-linter .
```

---

- **Runs once**
- **Working directory is the given subdirectory**
- **No path argument**

```toml
[commands.some-linter]
invoke = "once"
working-dir.chdir-to = "pkg1"
path-args = "none"
```

```
cd /example/pkg1
some-linter
```
