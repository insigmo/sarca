use sqlx::PgPool;

use crate::{
    common::password_manager::PasswordManager,
    config::Config,
    errors::SarcaResult,
    models::users::InDBUser,
    repositories::users::UsersRepository,
    schemas::users::InUser,
    services::auth::AuthService,
};

pub struct UsersService<'d> {
    repo: UsersRepository<'d>,
    db: &'d PgPool,
}

impl<'d> UsersService<'d> {
    pub fn new(db: &'d PgPool) -> Self {
        Self {
            repo: UsersRepository::new(db),
            db,
        }
    }

    pub async fn create(&self, in_user: InUser, config: &Config) -> SarcaResult<()> {
        let password_hash = PasswordManager::generate(&in_user.password).unwrap();
        let email = in_user.email.clone();
        let user = InDBUser::new_password(in_user.email, password_hash);
        let created = self.repo.create(user).await?;
        AuthService::new(self.db)
            .send_verify_email_soft(created.id, &email, config)
            .await;
        Ok(())
    }
}
