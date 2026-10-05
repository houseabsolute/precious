#!/usr/bin/env perl

use strict;
use warnings;

use File::Path qw( make_path );
use File::Spec;

symlink_hook('pre-commit');

sub symlink_hook {
    my $hook = shift;

    # In a git worktree, `.git` is a file, not a directory. Hooks are shared by all worktrees and
    # live in the main checkout's git dir, so we ask git where that is.
    my $common_dir = `git rev-parse --path-format=absolute --git-common-dir`;
    die "Could not find the git dir. Is this a git checkout?\n"
        if $? != 0;
    chomp $common_dir;

    my $hooks_dir = File::Spec->catdir( $common_dir, 'hooks' );
    my $dot       = File::Spec->catfile( $hooks_dir, $hook );

    # This is relative to the hooks dir, so it always points at the script in the main checkout,
    # even when this is run from a worktree. A worktree can be removed later, and a link to its
    # copy of the script would then be broken.
    my $link = "../../git/hooks/$hook.sh";

    # We check for a symlink first because -e is false for a symlink with a missing target.
    if ( -l $dot ) {
        return if readlink $dot eq $link;
        die "You already have a hook at $dot!\n";
    }
    die "You already have a hook at $dot!\n"
        if -e $dot;

    make_path($hooks_dir);
    symlink $link, $dot
        or die "Could not create a symlink at $dot: $!\n";

    print "Installed the $hook hook at $dot\n";
}
