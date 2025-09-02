# sampo-github-bot

GitHub App server to inspect pull requests and automatically request Sampo changesets when needed.

> [!WARNING]
> For now, only detects changesets by the presence of `.sampo/changesets/*.md` files.

## Usage

Install the [GitHub App](https://github.com/apps/bruits-sampo) on your repository. It will automatically request changesets on new pull requests.

*TODO: Add a detailed usage guide, with screenshots*

## Development

### Configuration

Set the following environment variables:

- `WEBHOOK_SECRET`: webhook secret configured in the GitHub App
- `GITHUB_APP_ID`: GitHub App ID (numeric)
- `GITHUB_PRIVATE_KEY`: GitHub App private key (PEM format)
- `PORT` (optional): port to listen on. Defaults to `3000`
- `ADDR` (optional): full socket address, e.g. `0.0.0.0:8080`. Overrides `PORT`

### Deployment

The app is deployed on [Fly.io](https://fly.io) as a GitHub App and automatically handles webhook authentication and GitHub API access using JWT tokens and installation tokens.

### Run locally

```
GITHUB_APP_ID=... GITHUB_PRIVATE_KEY="-----BEGIN RSA PRIVATE KEY-----..." WEBHOOK_SECRET=... cargo run -p sampo-github-bot
```

Then configure a webhook to `http://localhost:3000/webhook` via a tunnel (e.g., `ngrok`) for `pull_request` events.
