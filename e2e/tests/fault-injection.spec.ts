import { execFileSync } from "node:child_process";
import { test, expect } from "@playwright/test";

/**
 * Fault injection: a gateway 5xx during cold-start bootstrap must show the
 * retryable session error page, never a false "signed out" redirect
 * (spec §12.8, ADR-0006 §1).
 *
 * The test stops the API container, loads the SPA through the live gateway and
 * then restarts the API and retries. It is skipped in the normal nine-journey
 * suite; run it through scripts/fault-injection.sh, which sets
 * E2E_FAULT_INJECTION and the Compose coordinates below.
 */

const faultMode = process.env.E2E_FAULT_INJECTION === "1";

function required(name: string): string {
  const value = process.env[name];
  if (!value) {
    throw new Error(
      `${name} is required; run through scripts/fault-injection.sh`,
    );
  }
  return value;
}

function compose(args: string[]): void {
  // Read the coordinates lazily: the normal nine-journey suite collects (and
  // skips) this file without any FAULT_* variables.
  const project = required("FAULT_COMPOSE_PROJECT");
  const envFile = required("FAULT_COMPOSE_ENV_FILE");
  const files = required("FAULT_COMPOSE_FILES").split(":").filter(Boolean);
  const secretsDir = required("FAULT_SECRETS_DIR");

  const arguments_ = [
    "compose",
    "--project-name",
    project,
    "--env-file",
    envFile,
  ];
  for (const file of files) arguments_.push("--file", file);
  arguments_.push("--profile", "local-db", ...args);
  execFileSync("docker", arguments_, {
    stdio: "inherit",
    env: { ...process.env, RS_ADMIN_SECRETS_DIR: secretsDir },
  });
}

async function waitForApi(): Promise<void> {
  for (let attempt = 0; attempt < 120; attempt += 1) {
    try {
      compose([
        "exec",
        "-T",
        "api",
        "curl",
        "-fsS",
        "http://127.0.0.1:8080/health/ready",
      ]);
      return;
    } catch {
      await new Promise((resolve) => setTimeout(resolve, 500));
    }
  }
  throw new Error("the API did not become ready again");
}

test("a gateway 5xx shows the retryable session error page", async ({
  page,
}) => {
  test.skip(!faultMode, "run through scripts/fault-injection.sh");
  test.setTimeout(180_000);

  compose(["stop", "api"]);
  try {
    // Give the gateway time to observe the stopped upstream.
    await new Promise((resolve) => setTimeout(resolve, 3_000));

    // Chromium may surface the edge 502 as a response or as a failed request
    // (the nginx error page carries no CORS headers). Either proves the
    // bootstrap transport failed, which is what the page must handle.
    let transportFailure = false;
    page.on("response", (response) => {
      if (
        response.url().includes("/api/v1/auth/refresh") &&
        response.status() >= 500
      ) {
        transportFailure = true;
      }
    });
    page.on("requestfailed", (request) => {
      if (request.url().includes("/api/v1/auth/refresh")) {
        transportFailure = true;
      }
    });

    await page.goto("/");

    await expect(page.getByText("Can't restore your session")).toBeVisible({
      timeout: 30_000,
    });
    expect(transportFailure).toBe(true);
    await expect(page.getByRole("button", { name: "Retry" })).toBeVisible();
    // A transient failure must not be reported as a signed-out session.
    await expect(page).not.toHaveURL(/\/sign-in/);
  } finally {
    compose(["start", "api"]);
    await waitForApi();
  }

  await page.getByRole("button", { name: "Retry" }).click();
  // With no refresh cookie, the recovered bootstrap ends on the sign-in page.
  await expect(page).toHaveURL(/\/sign-in/, { timeout: 30_000 });
});
