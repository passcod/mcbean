use leptos::prelude::*;

#[component]
pub fn Editor(
    content: String,
    on_save: Callback<String>,
    #[prop(into)] on_cancel: Callback<()>,
) -> impl IntoView {
    let (value, set_value) = signal(content);

    let save = move |_| {
        on_save.run(value.get());
    };

    let cancel = move |_| {
        on_cancel.run(());
    };

    view! {
        <div class="field">
            <div class="control">
                <textarea
                    class="textarea"
                    prop:value=value
                    on:input=move |ev| {
                        set_value.set(event_target_value(&ev));
                    }
                />
            </div>
        </div>
        <div class="field is-grouped">
            <div class="control">
                <button class="button is-success" on:click=save>
                    "Save"
                </button>
            </div>
            <div class="control">
                <button class="button is-light" on:click=cancel>
                    "Cancel"
                </button>
            </div>
        </div>
    }
}
