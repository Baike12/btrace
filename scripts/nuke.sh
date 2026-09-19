
#  used to nuke the dev environment for engineers

find . -name 'node_modules' -type d -prune -print -exec rm -rf '{}' \;
find . -name '.next' -type d -prune -print -exec rm -rf '{}' \;
# Only the pnpm bin shims. A bare `-iname "bin"` also matches langfuse-rs/bin/,
# which holds the Rust binary sources (server.rs, worker.rs) — that would delete
# source, not build output.
find . -path '*/node_modules/.bin' -type d -prune -print -exec rm -rf '{}' \;
find . -iname "dist" -type d -prune -print -exec rm -rf '{}' \;
find . -iname "out" -type d -prune -print -exec rm -rf '{}' \;
find . -iname ".turbo" -type d -prune -print -exec rm -rf '{}' \;
find . -iname "tsconfig.tsbuildinfo" -type f -prune -print -exec rm -rf '{}' \;

pnpm store prune

# Rust build output lives in langfuse-rs/target (14GB when built); cargo clean is
# the correct way to drop it.
cd langfuse-rs && cargo clean
