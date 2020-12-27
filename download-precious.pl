#!/usr/bin/env perl

use strict;
use warnings;

use File::Path qw( mkpath );
use File::Spec;
use File::Temp qw( tempdir );
use Getopt::Long;

sub main {
    my $to;
    GetOptions( 'to:s' => \$to );
    $to = File::Spec->catfile( File::Spec->curdir, 'bin', 'precious' )
        unless defined $to && length $to;

    my $dir = tempdir(CLEANUP => 1 );
    _get_latest_release($dir);
}

sub _get_latest_release {
    my $dir = shift;

}

my $http_get;
sub _http_get {
    return $http_get if $http_get;

    if ( eval { require HTTP::Tiny; 1 } && HTTP::Tiny::can_ssl() ) {
        return \&_http_tiny;
    }
    else if 
}

main();

LOCATION=$( \
    curl -s https://api.github.com/repos/houseabsolute/precious/releases/latest
    | grep "tag_name" \
    | awk '{print "https://github.com/houseabsolute/precious/archive/" substr($2, 2, length($2)-3) ".zip"}' \
)

curl -L -o ./bin/precious $LOCATION
