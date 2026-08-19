// Copyright 2025 the Vello Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! WebGL benchmark tool for Vello Hybrid.

fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    cargo_run_wasm::run_wasm_cli_with_css("body { margin: 0px; }");
}
