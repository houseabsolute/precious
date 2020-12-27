#!/usr/bin/env perl

# Even CentOS 6 has Perl 5.10. I can't imagine anyone running this with 5.8.x.
use 5.010000;

use strict;
use warnings;

use Archive::Tar;
use Getopt::Long;
use HTTP::Tinyish;
use Path::Tiny qw( cwd path tempdir );

sub main {
    my $to;
    my $help;
    GetOptions(
        'to:s' => \$to,
        'help' => \$help,
    );
    if ($help) {
        my $script = path($0)->basename;
        print <<"EOF";
$script

  --to    The directory in which the precious binary should be written. Defaults
          to ./bin.
  --tag   The tag to download. Defaults to the latest release.
  --help  You're reading it.

EOF
        exit 0;
    }

    $to
        = defined $to && length $to
        ? path( $to, 'precious' )
        : cwd()->child( 'bin', 'precious' );

    _get_latest_release($to);

    exit 0;
}

sub _get_latest_release {
    my $to = shift;

    my $tag     = _get_tag();
    my $tarball = _tarball_name();
    my $url
        = "https://github.com/houseabsolute/precious/releases/download/$tag/$tarball";

    my $client = _http_client();
    my $resp   = $client->request(
        'GET',
        $url,
    );

    unless ( $resp->{success} ) {
        die "Request for $url failed with status $resp->{status}\n";
    }

    my $dir      = tempdir();
    my $tempfile = $dir->child('precious.tar.gz');
    $tempfile->spew_raw( $resp->{content} );

    my $tar = Archive::Tar->new($tempfile)
        or die "Could not read tarball at $tempfile: $!\n";
    my $bin = _bin_name();
    $tar->extract_file( $bin, $to )
        or die "Could not extract $bin from $tarball to $to\n";

    return;
}

sub _get_tag {
    my $client = _http_client();
    my $url
        = 'https://api.github.com/repos/houseabsolute/precious/releases/latest';
    my $resp = $client->get($url);
    unless ( $resp->{success} ) {
        die "Request for $url failed with status $resp->{status}\n";
    }
    my ($tag) = $resp->{content} =~ /"tag_name"\s*:\s*"([^"]+)"/
        or die "Could not determine tag from $url response\n";
    return $tag;
}

my $http_client;

# Mostly copied from
# https://github.com/miyagawa/cpanminus/blob/devel/Menlo-Legacy/lib/Menlo/CLI/Compat.pm
sub _http_client {
    return $http_client if $http_client;

    my @try = qw( HTTPTiny Wget Curl LWP );

    my $backend;
    for my $try ( map {"HTTP::Tinyish::$_"} @try ) {
        if ( my $meta = HTTP::Tinyish->configure_backend($try) ) {
            if ( $try->supports('https') ) {
                $backend = $try;
                last;
            }
        }
    }

    unless ($backend) {
        die
            "Could not find an HTTP client that supports SSL. Tried Perl libraries, wget, and curl.\n";
    }

    return $http_client
        = $backend->new( agent => 'download-precious.pl', verify_SSL => 1 );
}

sub _tarball_name {
    my $os
        = $^O eq 'linux'   ? 'Linux'
        : $^O eq 'darwin'  ? 'Darwin'
        : $^O eq 'MSWin32' ? 'Windows'
        :   die "There is no compiled precious binary for this OS: $^O\n";

    return "precious-$os-x86_64.tar.gz";
}

sub _bin_name {
    return $^O eq 'MSWin32'
        ? 'precious.exe'
        : 'precious';
}

main();
