use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys::{MediaRecorder, MediaStream, 
                MediaStreamConstraints, HtmlElement, Url, SpeechRecognition, SpeechRecognitionEvent};
use yew::prelude::*;
use std::fmt::{self, Display};
use gloo_timers::callback::Interval;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"])]
    async fn invoke(cmd: &str, args: JsValue) -> JsValue;
}

#[derive(Serialize, Deserialize)]
struct GreetArgs<'a> {
    name: &'a str,
}

async fn init_recorder(
    recorder_handle: UseStateHandle<Option<MediaRecorder>>,
    status: UseStateHandle<RecordingStatus>,
    chunks: UseStateHandle<Vec<web_sys::Blob>>,
    video_ref: NodeRef,
) {
    let navigator = web_sys::window().unwrap().navigator();
    let media_devices = navigator.media_devices().unwrap();

    // ① Request both audio & video
    let mut constraints = MediaStreamConstraints::new();
    constraints.video(&JsValue::TRUE);
    constraints.audio(&JsValue::TRUE);

    let media_promise = media_devices
        .get_user_media_with_constraints(&constraints)
        .unwrap();

    match wasm_bindgen_futures::JsFuture::from(media_promise).await {
        Ok(js_stream) => {
            let stream: MediaStream = js_stream.unchecked_into();

            // ② Live preview in the <video> element
            if let Some(video_el) = video_ref.cast::<web_sys::HtmlVideoElement>() {
                video_el.set_src_object(Some(&stream));
                video_el.set_muted(true);
                let _ = video_el.play();
            }

            // ③ Create MediaRecorder on that same stream
            let recorder = MediaRecorder::new_with_media_stream(&stream).unwrap();

            // ondataavailable → collect blobs
            {
                let chunks_clone = chunks.clone();
                let on_data = Closure::wrap(Box::new(move |e: web_sys::BlobEvent| {
                    // e.data() is Option<web_sys::Blob>, so just unwrap it
                    if let Some(blob) = e.data() {
                        let mut current = (*chunks_clone).clone();
                        current.push(blob);
                        chunks_clone.set(current);
                    }
                }) as Box<dyn FnMut(_)>);
                recorder.set_ondataavailable(Some(on_data.as_ref().unchecked_ref()));
                on_data.forget();
            }

            // onstop → update status
            {
                let status_clone = status.clone();
                let on_stop = Closure::wrap(Box::new(move || {
                    status_clone.set(RecordingStatus::Idle);
                }) as Box<dyn FnMut()>);
                recorder.set_onstop(Some(on_stop.as_ref().unchecked_ref()));
                on_stop.forget();
            }

            recorder_handle.set(Some(recorder));
            status.set(RecordingStatus::Idle);
        }
        Err(err) => {
            gloo::console::error!("getUserMedia error:", err);
        }
    }
}

#[derive(Clone, PartialEq)]
enum RecordingStatus {
    Idle,
    Recording,
    Paused,
}
impl Display for RecordingStatus {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                RecordingStatus::Recording => "Recording",
                RecordingStatus::Paused => "Paused",
                RecordingStatus::Idle => "Idle",
            }
        )
    }
}

#[function_component(App)]
pub fn app() -> Html {
    // Live WPM using the browser speech to text API
    let wpm = use_state(|| 120u32);
    let recog_ref = use_mut_ref(|| None::<web_sys::SpeechRecognition>);
        // on-mount: start recognition once
    let wpm_recog = wpm.clone();
    use_effect_with((), move |_| {
        if let Ok(recog) = SpeechRecognition::new() {
            web_sys::console::log_1(&"⚡ SR effect mounted".into());
            // Try to construct SR and log success or failure
            match SpeechRecognition::new() {
                Ok(recog) => {
                    web_sys::console::log_1(&"✅ SpeechRecognition::new() succeeded".into());
                    // … your existing setup for recog …
                }
                Err(err) => {
                    web_sys::console::error_1(&format!("❌ SpeechRecognition::new() failed: {:?}", err).into());
                }
            }
            // configure it
            recog.set_continuous(true);
            recog.set_interim_results(true);
    
            // stash it in our ref so we can stop it later
            recog_ref.borrow_mut().replace(recog.clone());
    
            // time markers
            let start_time = js_sys::Date::now();
    
            // onresult handler
            let on_result = Closure::wrap(Box::new(move |e: SpeechRecognitionEvent| {
                let mut transcript = String::new();
                let results = e.results() .expect("SpeechRecognitionEvent should always have results");
                // print the results
                web_sys::console::log_1(&format!("Results: {:?}", results).into());
                for i in 0..results.length() {
                    let res = results.get(i).unwrap();
                    transcript.push_str(&res.get(0).unwrap().transcript());
                    transcript.push(' ');
                }
                web_sys::console::log_1(&format!("Transcript so far: “{}”", transcript).into());

                let words   = transcript.split_whitespace().count() as f64;
                let elapsed = (js_sys::Date::now() - start_time) / 1000.0;
                if elapsed > 1.0 {
                    let current_wpm = (words / elapsed) * 60.0;
                    wpm_recog.set(current_wpm.round() as u32);
                }
            }) as Box<dyn FnMut(_)>);
    
            recog.set_onresult(Some(on_result.as_ref().unchecked_ref()));
            on_result.forget();
    
            // start recognition
            let _ = recog.start();
        }
    
        // **Remember**: only two arguments to use_effect_with,
        // so we return our teardown from inside this one closure:
        move || {
            if let Some(r) = recog_ref.borrow_mut().take() {
                let _ = r.stop();
            }
        }
    });
    // refs & state
    let video_ref = use_node_ref();
    let playback_url = use_state(|| None::<String>);
    let recorder_handle = use_state(|| None::<MediaRecorder>);
    let status = use_state(|| RecordingStatus::Idle);
    let chunks = use_state(Vec::new);

    let script = use_state(|| String::new());
    let is_prompting = use_state(|| false);
    let tele_ref = use_node_ref();
    // handler to start/stop the teleprompter
    let onclick_toggle = {
        let is_prompting = is_prompting.clone();
        Callback::from(move |_| {
            is_prompting.set(!*is_prompting);
        })
    };

    // initialize recorder + preview on mount
    {
        let recorder_handle = recorder_handle.clone();
        let status = status.clone();
        let chunks = chunks.clone();
        let video_ref = video_ref.clone();
        // a ref for the teleprompter div
        let tele_ref_for_effect = tele_ref.clone();
        use_effect_with(
            (*is_prompting, *wpm, (*script).clone()),
            move |(start, wpm_val, _script)| {
                // build optional interval
                let maybe_interval: Option<Interval> = if *start {
                    // compute bytes-per-ms
                    let words_per_ms = *wpm_val as f64 / 60_000.0;
                    // grab the element
                    let tele_el = tele_ref_for_effect
                        .cast::<HtmlElement>()
                        .expect("tele_ref must be a HtmlElement");
    
                    // accumulator in closure
                    let mut acc = 0.0;
                    // create the interval
                    Some(Interval::new(50, move || {
                        acc += words_per_ms * 50.0;
                        tele_el.set_scroll_top((acc * 20.0) as i32);
                    }))
                } else {
                    None
                };
    
                // always return *one* cleanup closure
                move || {
                    if let Some(interval) = maybe_interval {
                        drop(interval);
                    }
                }
            },
        ); 
        use_effect_with((), move |_| {
            // spawn your recorder init exactly once
            spawn_local(init_recorder(
                recorder_handle.clone(),
                status.clone(),
                chunks.clone(),
                video_ref.clone(),
            ));
            // return a no-op tear-down
            || ()
        });
    }

    // button callbacks
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
        let playback_url = playback_url.clone();
        Callback::from(move |_| {
            if let Some(rec) = recorder_handle.as_ref() {
                rec.stop().unwrap();
            }
            // After onstop fires and status becomes Idle, merge blobs
            if matches!(*status, RecordingStatus::Idle) {
                // Merge blobs into one video blob
                let arr = js_sys::Array::new();
                for blob in chunks.iter() {
                    arr.push(blob);
                }
                if let Ok(final_blob) = web_sys::Blob::new_with_blob_sequence(&arr) {
                    let url = Url::create_object_url_with_blob(&final_blob).unwrap();
                    playback_url.set(Some(url));
                }
            }
        })
    };

    html! {
        <main class="container">
            <h1>{"Born-Edited Recorder (Audio+Video)"}</h1>
            <p>{ format!("Live WPM: {}", *wpm) }</p>

            <div style="margin-bottom: 12px; display: flex; gap: 8px;">
            <textarea
              value={(*script).clone()}
              oninput={Callback::from({
                let script = script.clone();
                move |e: InputEvent| {
                  let txt = e.target_unchecked_into::<web_sys::HtmlTextAreaElement>().value();
                  script.set(txt);
                }
              })}
              placeholder="Paste your dialog script here…"
              style="flex:1; height: 80px;"
            />
            <button onclick={onclick_toggle.clone()}>
              { if *is_prompting { "Stop Teleprompter" } else { "Start Teleprompter" } }
            </button>
          </div>
        
          <div
          id="teleprompter"
          ref={tele_ref.clone()}
          style="
            width: 640px;
            height: 120px;            /* fixed height for ~8 lines of text */
            overflow-y: hidden;       /* hide overflow so we scroll within it */
            background: rgba(0,0,0,0.8);
            color: white;
            font-size: 24px;
            line-height: 1.4;
            padding: 8px;
            border-radius: 4px;
          "
        >
          { for script.split_whitespace().map(|w| html!{<span>{format!("{} ", w)}</span>}) }
        </div>
            // 1️⃣ Live webcam preview
            <video ref={video_ref.clone()} width="640" height="480" autoplay=true playsinline=true />

            <div class="controls">
                <p>{ format!("Status: {}", *status) }</p>
                <button onclick={onclick_start.clone()} disabled={!matches!(*status, RecordingStatus::Idle)}>{"Record"}</button>
                <button onclick={onclick_pause.clone()} disabled={!matches!(*status, RecordingStatus::Recording)}>{"Pause"}</button>
                <button onclick={onclick_stop.clone()} disabled={matches!(*status, RecordingStatus::Recording)}>{"Stop & Preview"}</button>
            </div>
            // 2️⃣ Playback of the recorded video
            {
                if let Some(url) = &*playback_url {
                    html! {
                        <video src={url.clone()} width="640" height="480" controls=true />
                    }
                } else {
                    html! {}
                }
            }
        </main>
    }
}
