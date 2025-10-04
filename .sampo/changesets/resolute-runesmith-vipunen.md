---
sampo-github-action: minor
---

**⚠️ breaking change:** Drop the built-in binary build/upload flags in favor of a simple new `release-assets` input. Workflows should now provide pre-built artifacts (paths or glob patterns, with optional renames and templated placeholders) that the action uploads after publishing.

```diff
      - name: Install Rust targets for cross-compilation
        run: rustup target add x86_64-apple-darwin x86_64-pc-windows-msvc

+      - name: Build binaries
+        run: |
+          cargo build --release --target x86_64-unknown-linux-gnu
+          cargo build --release --target x86_64-apple-darwin
+          cargo build --release --target x86_64-pc-windows-msvc
+          mkdir -p dist
+          tar -C target/x86_64-unknown-linux-gnu/release -czf dist/my-cli-x86_64-unknown-linux-gnu.tar.gz my-cli
+          tar -C target/x86_64-apple-darwin/release -czf dist/my-cli-x86_64-apple-darwin.tar.gz my-cli
+          zip -j dist/my-cli-x86_64-pc-windows-msvc.zip target/x86_64-pc-windows-msvc/release/my-cli.exe

      - name: Run Sampo to release & publish
        uses: bruits/sampo/crates/sampo-github-action@main
        with:
          create-github-release: true
-          upload-binary: true
-          targets: x86_64-unknown-linux-gnu, x86_64-apple-darwin, x86_64-pc-windows-msvc
+          release-assets: |
+            dist/my-cli-x86_64-unknown-linux-gnu.tar.gz => my-cli-{{tag}}-x86_64-unknown-linux-gnu.tar.gz
+            dist/my-cli-x86_64-apple-darwin.tar.gz => my-cli-{{tag}}-x86_64-apple-darwin.tar.gz
+            dist/my-cli-x86_64-pc-windows-msvc.zip => my-cli-{{tag}}-x86_64-pc-windows-msvc.zip
```
