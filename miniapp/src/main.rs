//! Miniapp s21notify: регистрация (пароль → offline-токен, не сохраняется),
//! настройки уведомлений, статус подключения. Работает внутри Telegram и MAX.

use gloo_net::http::{Request, RequestBuilder};
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Deserialize;
use serde_json::{json, Value};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    /// JS-глю из index.html: JSON {messenger, init_data} либо "".
    #[wasm_bindgen(js_name = platformDetect)]
    fn platform_detect() -> String;
}

// ------------------------------------------------------------------- типы API

#[derive(Clone, Debug, Deserialize)]
struct AuthResp {
    token: String,
    registered: bool,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct Linked {
    messenger: String,
    status: String,
    this_one: bool,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct Me {
    registered: bool,
    #[serde(default)]
    s21_login: String,
    #[serde(default)]
    token_status: String,
    #[serde(default)]
    linked: Vec<Linked>,
    #[serde(default)]
    last_poll_at: Option<String>,
    #[serde(default)]
    last_poll_error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct Settings {
    remind_minutes: String,
    notify_bookings: bool,
    notify_changes: bool,
    notify_reminders: bool,
    notify_feed: bool,
    notify_deadlines: bool,
    notify_alarm: bool,
}

#[derive(Clone, PartialEq)]
enum View {
    Loading,
    /// Не из мессенджера / фатальная ошибка
    Dead(String),
    /// Форма входа; true = перелогин (токен отозван)
    Login {
        relogin: bool,
    },
    Home,
}

// ---------------------------------------------------------------- API-клиент

#[derive(Clone, Copy)]
struct Ctx {
    view: RwSignal<View>,
    jwt: RwSignal<Option<String>>,
    messenger: RwSignal<String>,
    me: RwSignal<Me>,
    settings: RwSignal<Settings>,
    error: RwSignal<String>,
    busy: RwSignal<bool>,
}

fn bearer(req: RequestBuilder, jwt: &Option<String>) -> RequestBuilder {
    match jwt {
        Some(t) => req.header("Authorization", &format!("Bearer {t}")),
        None => req,
    }
}

async fn api_get(path: &str, jwt: &Option<String>) -> Result<Value, String> {
    let resp = bearer(Request::get(path), jwt)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    parse_resp(resp).await
}

async fn api_send(
    method: &str,
    path: &str,
    body: Value,
    jwt: &Option<String>,
) -> Result<Value, String> {
    let builder = match method {
        "PUT" => Request::put(path),
        "DELETE" => Request::delete(path),
        _ => Request::post(path),
    };
    let resp = bearer(builder, jwt)
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .map_err(|e| e.to_string())?
        .send()
        .await
        .map_err(|e| e.to_string())?;
    parse_resp(resp).await
}

async fn parse_resp(resp: gloo_net::http::Response) -> Result<Value, String> {
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if (200..300).contains(&status) {
        Ok(body)
    } else {
        Err(body["error"]
            .as_str()
            .unwrap_or("что-то пошло не так")
            .to_string())
    }
}

/// Полная загрузка: детект платформы → /api/auth → /api/me (+настройки).
async fn bootstrap(ctx: Ctx) {
    let detected = platform_detect();
    if detected.is_empty() {
        ctx.view.set(View::Dead(
            "Открой эту страницу из бота в Telegram или MAX 🙂".into(),
        ));
        return;
    }
    let parsed: Value = serde_json::from_str(&detected).unwrap_or(Value::Null);
    let messenger = parsed["messenger"].as_str().unwrap_or("").to_string();
    let init_data = parsed["init_data"].as_str().unwrap_or("").to_string();
    ctx.messenger.set(messenger.clone());

    let auth = match api_send(
        "POST",
        "/api/auth",
        json!({"messenger": messenger, "init_data": init_data}),
        &None,
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            ctx.view.set(View::Dead(format!("Не удалось войти: {e}")));
            return;
        }
    };
    let auth: AuthResp = match serde_json::from_value(auth) {
        Ok(a) => a,
        Err(_) => {
            ctx.view.set(View::Dead("Странный ответ сервера".into()));
            return;
        }
    };
    ctx.jwt.set(Some(auth.token));
    if !auth.registered {
        ctx.view.set(View::Login { relogin: false });
        return;
    }
    reload_home(ctx).await;
}

async fn reload_home(ctx: Ctx) {
    let jwt = ctx.jwt.get_untracked();
    match api_get("/api/me", &jwt).await {
        Ok(v) => {
            let me: Me = serde_json::from_value(v).unwrap_or_default();
            if !me.registered {
                ctx.view.set(View::Login { relogin: false });
                return;
            }
            let needs_relogin = me.token_status == "needs_relogin";
            ctx.me.set(me);
            if let Ok(s) = api_get("/api/settings", &jwt).await {
                ctx.settings
                    .set(serde_json::from_value(s).unwrap_or_default());
            }
            if needs_relogin {
                ctx.view.set(View::Login { relogin: true });
            } else {
                ctx.view.set(View::Home);
            }
        }
        Err(e) => ctx.view.set(View::Dead(format!("Ошибка загрузки: {e}"))),
    }
}

fn confirm(msg: &str) -> bool {
    web_sys::window()
        .and_then(|w| w.confirm_with_message(msg).ok())
        .unwrap_or(false)
}

// -------------------------------------------------------------------- вьюхи

#[component]
fn App() -> impl IntoView {
    let ctx = Ctx {
        view: RwSignal::new(View::Loading),
        jwt: RwSignal::new(None),
        messenger: RwSignal::new(String::new()),
        me: RwSignal::new(Me::default()),
        settings: RwSignal::new(Settings::default()),
        error: RwSignal::new(String::new()),
        busy: RwSignal::new(false),
    };
    spawn_local(bootstrap(ctx));

    move || match ctx.view.get() {
        View::Loading => view! { <div class="center">"Загрузка…"</div> }.into_any(),
        View::Dead(msg) => view! { <div class="center">{msg}</div> }.into_any(),
        View::Login { relogin } => view! { <LoginForm ctx=ctx relogin=relogin/> }.into_any(),
        View::Home => view! { <Home ctx=ctx/> }.into_any(),
    }
}

#[component]
fn LoginForm(ctx: Ctx, relogin: bool) -> impl IntoView {
    let login = RwSignal::new(String::new());
    let password = RwSignal::new(String::new());

    let submit = move |_| {
        let l = login.get();
        let p = password.get();
        if l.trim().is_empty() || p.is_empty() {
            ctx.error.set("Заполни логин и пароль".into());
            return;
        }
        ctx.busy.set(true);
        ctx.error.set(String::new());
        spawn_local(async move {
            let jwt = ctx.jwt.get_untracked();
            match api_send(
                "POST",
                "/api/credentials",
                json!({"login": l.trim(), "password": p}),
                &jwt,
            )
            .await
            {
                Ok(v) => {
                    password.set(String::new());
                    if let Some(t) = v["token"].as_str() {
                        ctx.jwt.set(Some(t.to_string()));
                    }
                    reload_home(ctx).await;
                }
                Err(e) => ctx.error.set(e),
            }
            ctx.busy.set(false);
        });
    };

    view! {
        <div class="card">
            <h1>"s21notify"</h1>
            {if relogin {
                view! {
                    <p class="warn-badge">
                        "⚠️ Платформа отозвала доступ (обычно после смены пароля). Войди заново."
                    </p>
                }.into_any()
            } else {
                view! {
                    <p class="hint">
                        "Уведомления платформы Школы 21: записи на проверку, напоминания с будильником, дедлайны и лента."
                    </p>
                }.into_any()
            }}
        </div>
        <div class="card">
            <h2>"Вход через платформу"</h2>
            <input type="text" placeholder="Логин (короткий ник)"
                prop:value=login
                on:input=move |ev| login.set(event_target_value(&ev)) />
            <input type="password" placeholder="Пароль платформы"
                prop:value=password
                on:input=move |ev| password.set(event_target_value(&ev)) />
            <p class="hint">
                "Пароль не сохраняется: он нужен один раз, чтобы платформа выдала сервису долгоживущий токен доступа. Отозвать можно сменой пароля."
            </p>
            {move || {
                let e = ctx.error.get();
                (!e.is_empty()).then(|| view! { <div class="error">{e}</div> })
            }}
            <button on:click=submit disabled=move || ctx.busy.get()>
                {move || if ctx.busy.get() { "Проверяю…" } else { "Войти" }}
            </button>
        </div>
    }
}

#[component]
fn Home(ctx: Ctx) -> impl IntoView {
    let save = move |_| {
        ctx.busy.set(true);
        ctx.error.set(String::new());
        spawn_local(async move {
            let s = ctx.settings.get_untracked();
            let jwt = ctx.jwt.get_untracked();
            let body = json!({
                "remind_minutes": s.remind_minutes,
                "notify_bookings": s.notify_bookings,
                "notify_changes": s.notify_changes,
                "notify_reminders": s.notify_reminders,
                "notify_feed": s.notify_feed,
                "notify_deadlines": s.notify_deadlines,
                "notify_alarm": s.notify_alarm,
            });
            match api_send("PUT", "/api/settings", body, &jwt).await {
                Ok(v) => {
                    if let Some(norm) = v["remind_minutes"].as_str() {
                        ctx.settings.update(|s| s.remind_minutes = norm.to_string());
                    }
                }
                Err(e) => ctx.error.set(e),
            }
            ctx.busy.set(false);
        });
    };

    let unlink = move |_| {
        if !confirm("Отвязать этот мессенджер? Уведомления сюда приходить перестанут.")
        {
            return;
        }
        spawn_local(async move {
            let jwt = ctx.jwt.get_untracked();
            let _ = api_send("POST", "/api/unlink", json!({}), &jwt).await;
            ctx.view
                .set(View::Dead("Мессенджер отвязан. Можно закрыть окно.".into()));
        });
    };

    let delete_account = move |_| {
        if !confirm("Удалить аккаунт целиком? Все привязки и настройки будут стёрты.")
        {
            return;
        }
        if !confirm("Точно-точно? Отменить будет нельзя.") {
            return;
        }
        spawn_local(async move {
            let jwt = ctx.jwt.get_untracked();
            let _ = api_send("DELETE", "/api/account", json!({}), &jwt).await;
            ctx.view
                .set(View::Dead("Аккаунт удалён. Спасибо, что пробовал!".into()));
        });
    };

    let toggle = move |get: fn(&Settings) -> bool, set: fn(&mut Settings, bool)| {
        let s = ctx.settings;
        view! {
            <input class="switch" type="checkbox"
                prop:checked=move || get(&s.get())
                on:change=move |ev| s.update(|st| set(st, event_target_checked(&ev))) />
        }
    };

    view! {
        <div class="card">
            <h1>{move || format!("👤 {}", ctx.me.get().s21_login)}</h1>
            {move || {
                let me = ctx.me.get();
                match (me.token_status.as_str(), &me.last_poll_error) {
                    ("ok", None) => view! {
                        <p class="ok-badge">
                            {format!("✅ Подключено · последний опрос: {}",
                                me.last_poll_at.as_deref().unwrap_or("ещё не было").replace('T', " "))}
                        </p>
                    }.into_any(),
                    ("ok", Some(err)) => view! {
                        <p class="warn-badge">{format!("⚠️ Ошибка опроса: {err}")}</p>
                    }.into_any(),
                    _ => view! { <p class="warn-badge">"⚠️ Нужен повторный вход"</p> }.into_any(),
                }
            }}
            <div>
                {move || ctx.me.get().linked.iter().map(|l| {
                    let mark = match l.status.as_str() {
                        "active" => "🟢",
                        "not_started" => "⚪",
                        _ => "🔴",
                    };
                    let name = if l.messenger == "telegram" { "Telegram" } else { "MAX" };
                    let me_mark = if l.this_one { " (это окно)" } else { "" };
                    view! { <div class="linked"><span>{format!("{mark} {name}{me_mark}")}</span><span class="hint">{l.status.clone()}</span></div> }
                }).collect::<Vec<_>>()}
            </div>
        </div>

        <div class="card">
            <h2>"Уведомления"</h2>
            <div class="row"><label>"🔔 Новые записи на проверку"</label>
                {toggle(|s| s.notify_bookings, |s, v| s.notify_bookings = v)}</div>
            <div class="row"><label>"🔁 Переносы и отмены"</label>
                {toggle(|s| s.notify_changes, |s, v| s.notify_changes = v)}</div>
            <div class="row"><label>"⏰ Напоминания перед проверкой"</label>
                {toggle(|s| s.notify_reminders, |s, v| s.notify_reminders = v)}</div>
            <div class="row"><label>"🏫 Лента платформы"</label>
                {toggle(|s| s.notify_feed, |s, v| s.notify_feed = v)}</div>
            <div class="row"><label>"📅 Дедлайны и экзамены"</label>
                {toggle(|s| s.notify_deadlines, |s, v| s.notify_deadlines = v)}</div>
            <div class="row"><label>"🚨 Будильник, если не подтвердил"</label>
                {toggle(|s| s.notify_alarm, |s, v| s.notify_alarm = v)}</div>

            <label class="hint">"Пороги напоминаний, минут до проверки:"</label>
            <input type="text"
                prop:value=move || ctx.settings.get().remind_minutes
                on:input=move |ev| ctx.settings.update(|s| s.remind_minutes = event_target_value(&ev)) />
            {move || {
                let e = ctx.error.get();
                (!e.is_empty()).then(|| view! { <div class="error">{e}</div> })
            }}
            <button on:click=save disabled=move || ctx.busy.get()>
                {move || if ctx.busy.get() { "Сохраняю…" } else { "Сохранить" }}
            </button>
        </div>

        <div class="card">
            <button class="secondary" on:click=unlink>"Отвязать этот мессенджер"</button>
            <div style="height:8px"></div>
            <button class="danger" on:click=delete_account>"Удалить аккаунт полностью"</button>
        </div>
    }
}

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}
