#!/usr/bin/env bash
# Measure linked, stripped artifact sizes for the embedded-size wrapper.
# usage: tools/embedded-size/measure.sh <rust target triple>
# Prints a markdown table. Knobs are set per row via CARGO_PROFILE_RELEASE_*.
# Linked sizes: iOS = tiny C app linked against the staticlib with
# `clang -Os -dead_strip` then stripped; other targets = the Rust bin, stripped.
set -u
T=$1
cd "$(dirname "$0")"
TC=${TC:-1.94.0}
NT=${NIGHTLY:-nightly}

size_profile() { # $1 = opt-level, $2 = lto
  echo CARGO_PROFILE_RELEASE_OPT_LEVEL=$1 CARGO_PROFILE_RELEASE_LTO=${2:-fat} CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 CARGO_PROFILE_RELEASE_PANIC=abort CARGO_PROFILE_RELEASE_STRIP=symbols
}
fsize() { if [ "$(uname)" = Darwin ]; then stat -f%z "$1"; else stat -c%s "$1"; fi; }
stripped() {
  cp "$1" /tmp/es.$$
  if [ "$(uname)" = Darwin ]; then
    case "$1" in *.dylib) strip -x /tmp/es.$$ ;; *) strip /tmp/es.$$ 2>/dev/null || strip -x /tmp/es.$$ ;; esac
  else strip /tmp/es.$$; fi
  fsize /tmp/es.$$; rm -f /tmp/es.$$
}
app_c() { # C driver that references only the enabled chains' entry points
  local feats=$1 decl="" body=""
  for pair in eth:vs_ethereum tron:vs_tron unspec:vs_unspecified near:vs_near sui:vs_sui solana:vs_solana; do
    f=${pair%%:*}; s=${pair##*:}
    case ",$feats," in *",$f,"*) decl="$decl char *$s(const char *);"; body="$body if(!strcmp(argv[1],\"$f\")) r=$s(argv[2]);";; esac
  done
  printf '#include <stdio.h>\n#include <string.h>\n%s void vs_free(char*);\nint main(int argc,char**argv){char*r=0; if(argc<3)return 1;%s if(r){puts(r);vs_free(r);} return 0;}\n' "$decl" "$body"
}
measure() { # label toolchain env feats [extra cargo args...]
  local label=$1 tc=$2 envs=$3 feats=$4; shift 4
  local d=target/$T/release out lib="" cd=""
  if ! out=$(env $envs RUSTFLAGS="${RF:-}" cargo +$tc build --release --no-default-features --features "$feats" --target $T "$@" 2>&1); then
    echo "| $label | $T | ${feats:-none} | build failed: $(echo "$out" | grep -m1 '^error' | cut -c1-100) | | |"; return; fi
  case $T in
    *-apple-ios*)
      app_c "$feats" > /tmp/es_app.c
      local sdk=iphoneos; [[ $T == *sim ]] && sdk=iphonesimulator
      if xcrun -sdk $sdk clang -arch arm64 -Os -dead_strip /tmp/es_app.c $d/libembedded_size.a -o /tmp/es_app \
           -framework Security -framework CoreFoundation -lc++ 2>/tmp/es_link.err; then lib=$(stripped /tmp/es_app)
      else lib="link failed: $(head -c 100 /tmp/es_link.err)"; fi ;;
    *) lib=$(stripped $d/embedded-size-bin) ;;
  esac
  for f in $d/libembedded_size.dylib $d/libembedded_size.so; do [ -f $f ] && cd=$(stripped $f); done
  echo "| $label | $T | ${feats:-none} | $lib | $cd | $(fsize $d/libembedded_size.a) |"
}

echo "| config | target | chains | linked app/bin (stripped) | cdylib (stripped) | staticlib .a (unlinked) |"
echo "|---|---|---|---:|---:|---:|"
ALL=eth,tron,unspec
measure "default release" $TC "" $ALL
measure "size profile (opt z)" $TC "$(size_profile z)" $ALL
measure "size profile (opt s)" $TC "$(size_profile s)" $ALL
for f in eth tron unspec near sui; do measure "size profile (opt z)" $TC "$(size_profile z)" $f; done
measure "size profile (opt z, thin LTO)" $TC "$(size_profile z thin)" solana
if rustup toolchain list | grep -q "$NT"; then
  for f in $ALL eth; do
    RF="-Zunstable-options -Cpanic=immediate-abort" measure "nightly build-std, panic=immediate-abort" $NT "$(size_profile z)" "$f" -Z build-std=std,panic_abort
  done
fi
