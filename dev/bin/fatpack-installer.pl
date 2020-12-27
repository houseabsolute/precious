#!/bin/bash

set -e
set -x

# By default fatpack includes anything that is loaded from site_perl. That
# means if we have newer versions of core modules like Archive::Tar or
# IO::Handle installed there, they get fatpacked. This scripts creates a clean
# new perlbrew lib and installs just the needed prereqs for the script, then
# fatpacks with that lib. This ensures that our fatpacked script only include
# the minimum necessary modules.
perlbrew lib create perl-5.30.1@fatpack-precious-downloader
perlbrew exec --with perl-5.30.1@fatpack-precious-downloader 'cpanm -n App::FatPacker HTTP::Tinyish Path::Tiny'
perlbrew exec --with perl-5.30.1@fatpack-precious-downloader 'fatpack file ./dev/bin/install-precious.pl' > ./dev/bin/install-precious.packed.pl
perlbrew lib delete perl-5.30.1@fatpack-precious-downloader
