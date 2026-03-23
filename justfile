export DATABASE_URL := env('DATABASE_URL', 'postgresql://localhost/mcbean')
export DEV_USER_EMAIL := env('DEV_USER_EMAIL', 'test@test.nz')

# Install required dev tools
setup-tools:
    @command -v cargo-binstall >/dev/null 2>&1 || cargo install cargo-binstall --locked
    cargo binstall -y diesel_cli cargo-leptos cargo-nextest

# Run cargo-leptos in watch mode
watch:
    cargo leptos watch

# Refresh GraphQL schemas from upstream sources
refresh-schemas:
    curl -fsSL https://docs.github.com/public/fpt/schema.docs.graphql -o graphql/schema.graphql

# Run database migrations
migrate:
    diesel migration run

# Run tests with nextest
test:
    cargo nextest run

# Check SSR compilation (default features)
check-ssr:
    cargo check

# Check hydrate compilation
check-hydrate:
    cargo check --no-default-features --features hydrate

# Check all compilation targets
check: check-ssr check-hydrate

# Set up Tailscale funnel for dev (exposes port 5050)
funnel:
    tailscale funnel 5050
