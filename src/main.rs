// Copyright 2025 the Vello Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! WebGL benchmark tool for Vello Hybrid.

fn main() {
    #[cfg(target_arch = "wasm32")]
    {
        // In worker/child modes, dedicated entry points handle everything --
        // skip the normal UI startup.
        let is_worker = js_sys::Reflect::get(&js_sys::global(), &"__vello_worker".into())
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let is_ab_child = js_sys::Reflect::get(&js_sys::global(), &"__vello_ab_child".into())
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if is_worker {
            return;
        }
        if is_ab_child {
            return;
        }

        console_error_panic_hook::set_once();
        console_log::init_with_level(log::Level::Warn).unwrap();

        wasm_bindgen_futures::spawn_local(async move {
            vello_bench2::run().await;
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    cargo_run_wasm::run_wasm_cli_with_css("body { margin: 0px; }");
}
