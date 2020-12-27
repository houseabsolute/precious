#!/bin/bash

set -e
set -x

perlbrew lib create perl-5.30.1@fatpack-precious-downloader
perlbrew exec --with perl-5.30.1@fatpack-precious-downloader 'cpanm -n App::FatPacker HTTP::Tinyish Path::Tiny'
perlbrew exec --with perl-5.30.1@fatpack-precious-downloader 'fatpack file ./dev/bin/install-precious.pl' > ./dev/bin/install-precious.packed.pl
perlbrew lib delete perl-5.30.1@fatpack-precious-downloader
