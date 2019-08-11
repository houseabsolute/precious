set -ex

main() {
    # The Docker images used by cross don't have git installed so we cannot
    # run tests there. However, we do want to use them to build releases.
    if [ -z $RUN_TESTS ]; then
        cross build --target $TARGET
        cross build --target $TARGET --release
    else
        cargo build --verbose
        cargo test  --verbose -- --nocapture
    fi
}

# we don't run the "test phase" when doing deploys
if [ -z $TRAVIS_TAG ]; then
    main
fi
