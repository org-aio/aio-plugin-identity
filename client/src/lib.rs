mod http;
mod login;
mod profile;
mod registration;

pub use aio_plugin_identity_model::SessionView;
use az_dioxus_admin_shell::{ApplicationPage, ApplicationPlugin, ApplicationScene};
use dill::CatalogBuilder;
pub use http::load_session;
pub use login::LoginPage;

#[derive(Debug)]
pub struct IdentityPlugin;
impl ApplicationPlugin for IdentityPlugin {
    fn pages(&self) -> Vec<ApplicationPage> {
        vec![ApplicationPage {
            id: "profile",
            label: "个人资料",
            icon: Some("user"),
            scene: ApplicationScene {
                id: "system",
                label: "系统",
            },
            menu_path: Vec::new(),
            required_permission: None,
            render: profile::ProfilePage,
        }]
    }
}
pub fn register(builder: &mut CatalogBuilder) {
    builder
        .add_value(IdentityPlugin)
        .bind::<dyn ApplicationPlugin, IdentityPlugin>();
}
