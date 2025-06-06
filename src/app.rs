use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use yew::prelude::*;
use web_sys::{MediaRecorder, MediaStream};
use std::fmt::{self, Display};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"])]
    async fn invoke(cmd: &str, args: JsValue) -> JsValue;
}

#[derive(Serialize, Deserialize)]
struct GreetArgs<'a> {
    name: &'a str,
}

// 1️⃣ Define your helper function out here, above the component.
//    It’s asynchronous because `get_user_media()` returns a JS Promise.
async fn init_recorder(
    recorder_handle: UseStateHandle<Option<MediaRecorder>>,
    status: UseStateHandle<RecordingStatus>,
    chunks: UseStateHandle<Vec<web_sys::Blob>>,
) {
    // Request microphone access
    let navigator = web_sys::window().unwrap().navigator();
    let media_devices = navigator.media_devices().unwrap();
    let mut constraints = web_sys::MediaStreamConstraints::new();
    constraints.audio(&JsValue::TRUE);

    let js_stream = match wasm_bindgen_futures::JsFuture::from(
        media_devices.get_user_media_with_constraints(&constraints).unwrap()
    )
    .await
    {
        Ok(stream) => stream,
        Err(err) => {
            // Permission denied or other error
            gloo::console::error!("getUserMedia error:", err);
            return;
        }
    };

    // Cast it to a MediaStream and create a MediaRecorder
    let stream: MediaStream = js_stream.unchecked_into();
    let recorder = MediaRecorder::new_with_media_stream(&stream).unwrap();

    // Attach ondataavailable → push into `chunks`
    {
        let chunks_clone = chunks.clone();
        let on_data = Closure::wrap(Box::new(move |e: web_sys::BlobEvent| {
            if let Some(blob) = e.data().as_ref() {
                let mut current = (*chunks_clone).clone();
                current.push(blob.clone());
                chunks_clone.set(current);
            }
        }) as Box<dyn FnMut(_)>);
        recorder.set_ondataavailable(Some(on_data.as_ref().unchecked_ref()));
        on_data.forget();
    }

    // Attach onstop → update status back to Idle
    {
        let status_clone = status.clone();
        let on_stop = Closure::wrap(Box::new(move || {
            status_clone.set(RecordingStatus::Idle);
        }) as Box<dyn FnMut()>);
        recorder.set_onstop(Some(on_stop.as_ref().unchecked_ref()));
        on_stop.forget();
    }

    // Finally, store the recorder in state so you can call .start(), .pause(), .stop()
    recorder_handle.set(Some(recorder));
    status.set(RecordingStatus::Idle);
}

// 2️⃣ Your recording‐state enum
#[derive(Clone, PartialEq)]
enum RecordingStatus {
    Idle,
    Recording,
    Paused,
}

impl Display for RecordingStatus {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            // Match each variant of your enum and write its desired string representation
            RecordingStatus::Recording => write!(f, "Recording"),
            RecordingStatus::Paused => write!(f, "Paused"),
            RecordingStatus::Idle => write!(f, "Idle"),
            // ... and so on for other variants
        }
    }
}

#[function_component(App)]
pub fn app() -> Html {
    let greet_input_ref = use_node_ref();

    let name = use_state(|| String::new());

    let greet_msg = use_state(|| String::new());
    {
        let greet_msg = greet_msg.clone();
        let name = name.clone();
        let name2 = name.clone();
        use_effect_with(
            name2,
            move |_| {
                spawn_local(async move {
                    if name.is_empty() {
                        return;
                    }

                    let args = serde_wasm_bindgen::to_value(&GreetArgs { name: &*name }).unwrap();
                    // Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
                    let new_msg = invoke("greet", args).await.as_string().unwrap();
                    greet_msg.set(new_msg);
                });

                || {}
            },
        );
    }

    // A) UseState for storing the MediaRecorder instance
    let recorder_handle = use_state(|| None::<MediaRecorder>);
    // B) UseState for status (Idle / Recording / Paused)
    let status = use_state(|| RecordingStatus::Idle);
    // C) UseState for collected blobs
    let chunks = use_state(Vec::new);

    // 4️⃣ On mount, call init_recorder exactly once:
    {
        let recorder_handle = recorder_handle.clone();
        let status = status.clone();
        let chunks = chunks.clone();
        use_effect(move || {
            // Spawn the async init on mount
            spawn_local(init_recorder(recorder_handle, status, chunks));
            // Because we only want this to run once, return a no-op cleanup:
            || ()
        });
    }

    let onclick_start = {
        let recorder_handle = recorder_handle.clone();
        let status = status.clone();
        Callback::from(move |_| {
            if let Some(rec) = recorder_handle.as_ref() {
                rec.start().unwrap();
                status.set(RecordingStatus::Recording);
            }
        })
    };
    let onclick_pause = {
        let recorder_handle = recorder_handle.clone();
        let status = status.clone();
        Callback::from(move |_| {
            if let Some(rec) = recorder_handle.as_ref() {
                rec.pause().unwrap();
                status.set(RecordingStatus::Paused);
            }
        })
    };
    let onclick_stop = {
        let recorder_handle = recorder_handle.clone();
        let status = status.clone();
        let chunks = chunks.clone();
        Callback::from(move |_| {
            if let Some(rec) = recorder_handle.as_ref() {
                rec.stop().unwrap();
                // You could also merge blobs here into a final URL if desired.
                // For now, just set status back to Idle is handled by onstop.
                //
                // Example of merging (optional):
                // let array = js_sys::Array::new();
                // for blob in chunks.iter() {
                //     array.push(blob);
                // }
                // let final_blob = web_sys::Blob::new_with_blob_sequence(&array).unwrap();
                // let url = web_sys::Url::create_object_url_with_blob(&final_blob).unwrap();
                // ...store `url` in another state to render <audio> or <a download>...
            }
        })
    };

    let greet = {
        let name = name.clone();
        let greet_input_ref = greet_input_ref.clone();
        Callback::from(move |e: SubmitEvent| {
            e.prevent_default();
            name.set(
                greet_input_ref
                    .cast::<web_sys::HtmlInputElement>()
                    .unwrap()
                    .value(),
            );
        })
    };

    html! {
        <main class="container">
            <h1>{"Born Edited"}</h1>
            <div class="row">
                <form class="row" onsubmit={greet}>
                    <input id="greet-input" ref={greet_input_ref} placeholder="Enter a name..." />
                    <button type="submit">{"Greet"}</button>
                </form>
                <p>{ &*greet_msg }</p>
            </div>

            <div class="row">
                <p>{ "Status: " }{ &*status }</p>
                <button
                    onclick={onclick_start.clone()}
                    disabled={!matches!(*status, RecordingStatus::Idle)}
                >{ "Record" }</button>
                <button
                    onclick={onclick_pause.clone()}
                    disabled={!matches!(*status, RecordingStatus::Recording)}
                >{ "Pause" }</button>
                <button
                    onclick={onclick_stop.clone()}
                    disabled={matches!(*status, RecordingStatus::Idle)}
                >{ "Stop" }</button>
          </div>
        </main>
    }
}
