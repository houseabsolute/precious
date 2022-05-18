# dh-cargo fork

This is a slight fork of the debhelper script [dh-cargo],
with the following functional differences
since git commit 5cc7f7b
(included with version 28 released 2021-11-07):

  * support custom dh option --sourcedirectory
  * generate cargo-checksum during install
  * omit installing any .git* files (i.e. also e.g. .gitignore)
  * omit installing license files
  * omit installing debian/patches
  * strip checksums in Cargo.lock (not remove the whole file)

Also included is a slight fork of related cargo wrapper script,
with the following functional differences
since git commit 7823074
(included with version 0.57.0-6 released 2022-04-10):

  * fix support relative path in CARGO_HOME, as documented

[dh-cargo]: <https://salsa.debian.org/rust-team/dh-cargo/-/blob/master/cargo.pm>

[cargo]: <https://salsa.debian.org/rust-team/cargo/-/blob/debian/sid/debian/bin/cargo>


## Usage

In your source package,
copy directory `dh-cargo` to `debian/dh-cargo`
and edit `debian/rules` to something like this:

```
#!/usr/bin/make -f

# use local fork of dh-cargo and cargo wrapper
PATH := $(CURDIR)/debian/dh-cargo/bin:$(PATH)
PERL5LIB = $(CURDIR)/debian/dh-cargo/lib
export PATH PERL5LIB

%:
	dh $@ --buildsystem cargo --sourcedirectory=rustls
```


 -- Jonas Smedegaard <dr@jones.dk>  Wed, 18 May 2022 20:56:42 +0200
