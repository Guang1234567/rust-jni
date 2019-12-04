#!/bin/bash

set -e

echo "Running $0 $*..."

# error: dlopen(/${HOME}/dev_kit/src_code/rust-jni/examples/java-lib/dylib/target/debug/deps/librust_jni_generator-d87ff0567095b24e.dylib, 1): Library not loaded: @rpath/libjvm.dylib
#  Referenced from: /Users/lihanguang/dev_kit/src_code/rust-jni/examples/java-lib/dylib/target/debug/deps/librust_jni_generator-d87ff0567095b24e.dylib
export DYLD_FALLBACK_LIBRARY_PATH=${DYLD_FALLBACK_LIBRARY_PATH}:${JAVA_HOME}/jre/lib/server

(cd java && (rm rustjni/test/*.class || true) && javac rustjni/test/*.java)
(cd dylib/ && cargo build)
cargo test $*
