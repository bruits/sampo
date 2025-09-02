# sampo-github-bot

Minimal GitHub App server to inspect pull requests and ask for a Sampo changeset when needed.

> [!WARNING]
> For now, only detects changesets by the presence of `.sampo/changesets/*.md` files.

## Configuration

Set the following environment variables:

- `WEBHOOK_SECRET` (or `GITHUB_WEBHOOK_SECRET`): the webhook secret configured in the GitHub App.
- `GITHUB_TOKEN`: a token with permission to read PR files and create/update issue comments (installation token or PAT for testing).
- `PORT` (optional): port to listen on. Defaults to `3000`.
- `ADDR` (optional): full socket address, e.g. `0.0.0.0:8080`. Overrides `PORT`.

## Run locally

```
GITHUB_TOKEN=... WEBHOOK_SECRET=... cargo run -p sampo-github-bot
```

Then configure a webhook to `http://localhost:3000/webhook` via a tunnel (e.g., `ngrok`) for `pull_request` events.

TEST
