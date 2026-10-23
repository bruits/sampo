# Sampo's GitHub Bot

GitHub App server to inspect pull requests and automatically request Sampo changesets when needed.

Not sure what Sampo is? Don't know where to start? Check out Sampo's [documentation](https://github.com/bruits/sampo/blob/main/crates/sampo/README.md).

## Usage

Install the [GitHub App](https://github.com/apps/sampo-s-bot) on your repository. It will automatically request changesets on new pull requests.

## Development

The bot is an [`axum`](https://docs.rs/axum/latest/axum/) service configured only by environment variables: `WEBHOOK_SECRET`, `GITHUB_APP_ID`, `GITHUB_PRIVATE_KEY`, and optionally `PORT` and `ADDR`. Trying it locally takes a real GitHub App and a tunnel (e.g. `ngrok http 3000`) so GitHub reaches your machine. It talks to GitHub through [`octocrab`](https://docs.rs/octocrab/latest/octocrab/) and signs its tokens with [`jsonwebtoken`](https://docs.rs/jsonwebtoken/latest/jsonwebtoken/), so a refactor of its requests keeps rate limits and key handling in mind. A release deploys it to [Fly.io](https://fly.io) with `fly.toml`. A change to its configuration updates that file and the secrets set on Fly.io too.

Refer to [CONTRIBUTING.md](https://github.com/bruits/sampo/blob/main/CONTRIBUTING.md#getting-started) for development setup and workflow details.
