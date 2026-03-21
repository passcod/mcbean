use leptos::prelude::*;

#[component]
pub fn SpecViewer(content: String) -> impl IntoView {
    let lines: Vec<String> = content.lines().map(String::from).collect();

    view! {
        <div class="box">
            <pre class="content">
                {lines
                    .into_iter()
                    .map(|line| {
                        let has_rule_id = line.contains("[REQ-") || line.contains("[SPEC-");
                        if has_rule_id {
                            let tag_text = line
                                .split('[')
                                .nth(1)
                                .and_then(|s| s.split(']').next())
                                .unwrap_or("")
                                .to_string();
                            view! {
                                <span>
                                    {line.clone()}
                                    " "
                                    <span class="tag is-light is-small">{tag_text}</span>
                                </span>
                                <br />
                            }
                                .into_any()
                        } else {
                            view! {
                                <span>{line}</span>
                                <br />
                            }
                                .into_any()
                        }
                    })
                    .collect::<Vec<_>>()}
            </pre>
        </div>
    }
}
