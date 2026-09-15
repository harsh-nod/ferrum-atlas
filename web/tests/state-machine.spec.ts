import { expect, test } from "@playwright/test";
import { definitions, mockApi, snapshots } from "./fixtures";
import type {
  StateMachineInference,
  StateTransitionReview,
} from "../src/api/types";

const span = definitions[0].span;
const inference: StateMachineInference = {
  envelope: {
    algorithm_version: "state-machine/test",
    coverage: {
      status: "partial",
      reasons: [{ reason: "syntax_only", count: 1 }],
      limitations: ["Syntactic association is not a complete state machine"],
    },
    truncated: false,
    cancelled: false,
    deadline_reached: false,
    assumptions: [
      "Reviewer names are declarations, not authenticated identities",
    ],
  },
  input_digest: "state-machine-input:test",
  definition_id: definitions[0].id,
  enum_path: "State",
  state_place: "state",
  body_span: span,
  visited_nodes: 12,
  candidates: [
    {
      id: "candidate:test",
      from_variant: "Idle",
      to_variant: "Running",
      match_span: span,
      arm_span: span,
      assignment_span: span,
      guard: { span, text: "ready" },
      action: { span, text: "state = State::Running;" },
    },
  ],
  unknowns: [{ span, reason: "Calls may modify external state" }],
};

for (const width of [1440, 390])
  test(`state candidates and separate immutable reviewer declarations at ${width}`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width, height: 900 });
    await mockApi(page);
    const reviews: StateTransitionReview[] = [];
    let analyses = 0;
    await page.route("**/v1/analysis/state-machine/**", async (route) => {
      if (new URL(route.request().url()).pathname.endsWith("/reviews")) {
        if (route.request().method() === "GET")
          return route.fulfill({ json: reviews });
        const body = route.request().postDataJSON();
        expect(body.selection.snapshot_id).toBe(snapshots[0].id);
        expect(body.selection.context_id).toBe(snapshots[0].context.id);
        expect(body.review.input_digest).toBe(inference.input_digest);
        reviews.push(body.review);
        return route.fulfill({ json: body.review });
      }
      analyses++;
      expect(route.request().postDataJSON().enum_path).toBe("State");
      await route.fulfill({
        json: {
          api_version: "1",
          snapshot_id: snapshots[0].id,
          context_id: snapshots[0].context.id,
          analysis: inference,
        },
      });
    });
    await page.goto(
      "/?snapshot=snapshot%3Atwo&context=context%3Ahost&definition=definition%3Amain&view=flow#token=test-token",
    );
    if (width < 600)
      await page.getByRole("button", { name: "Show graph or flow" }).click();
    const panel = page.getByRole("region", { name: "State transition review" });
    await expect(panel).toBeVisible();
    expect(analyses).toBe(0);
    await panel.getByLabel("Enum path").fill("State");
    await panel.getByLabel("State variable").fill("state");
    await panel.getByRole("button", { name: "Analyze states" }).click();
    await expect(
      panel.getByText("Syntactic candidates / State / state"),
    ).toBeVisible();
    await expect(
      panel.getByRole("button", { name: "Accept candidate" }),
    ).toBeDisabled();
    await panel.getByLabel("Declared reviewer").fill("Local reviewer");
    await panel
      .getByLabel("Review note")
      .fill("Guard checked against this source only");
    await panel.getByRole("button", { name: "Accept candidate" }).click();
    await expect(
      panel.getByText(
        "accepted / Local reviewer: Guard checked against this source only",
      ),
    ).toBeVisible();
    await expect(
      panel.getByText("Syntactic association is not a complete state machine"),
    ).toBeVisible();
    await panel.getByText("Action source", { exact: true }).click();
    await expect(
      panel.getByText("state = State::Running;", { exact: true }),
    ).toBeVisible();
    expect(
      await panel
        .locator(".analysis-table-scroll")
        .evaluate((element) => element.scrollWidth <= element.clientWidth),
    ).toBe(true);
    await panel.screenshot({ path: testInfo.outputPath("state-review.png") });
    await page.reload();
    if (width < 600)
      await page.getByRole("button", { name: "Show graph or flow" }).click();
    await panel.getByLabel("Enum path").fill("State");
    await panel.getByLabel("State variable").fill("state");
    await panel.getByRole("button", { name: "Analyze states" }).click();
    await expect(
      panel.getByText(
        "accepted / Local reviewer: Guard checked against this source only",
      ),
    ).toBeVisible();
    expect(reviews).toHaveLength(1);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
  });

test("failed and stale review acknowledgements never appear accepted", async ({
  page,
}) => {
  await mockApi(page);
  let fail = true;
  await page.route("**/v1/analysis/state-machine/**", async (route) => {
    if (new URL(route.request().url()).pathname.endsWith("/reviews")) {
      if (route.request().method() === "GET")
        return route.fulfill({ json: [] });
      if (fail)
        return route.fulfill({
          status: 503,
          json: { code: "unavailable_evidence", message: "Review unavailable" },
        });
      return route.fulfill({
        json: {
          ...route.request().postDataJSON().review,
          input_digest: "other-revision",
        },
      });
    }
    await route.fulfill({
      json: {
        api_version: "1",
        snapshot_id: snapshots[0].id,
        context_id: snapshots[0].context.id,
        analysis: inference,
      },
    });
  });
  await page.goto(
    "/?snapshot=snapshot%3Atwo&context=context%3Ahost&definition=definition%3Amain&view=flow#token=test-token",
  );
  const panel = page.getByRole("region", { name: "State transition review" });
  await panel.getByLabel("Enum path").fill("State");
  await panel.getByLabel("State variable").fill("state");
  await panel.getByRole("button", { name: "Analyze states" }).click();
  await panel.getByLabel("Declared reviewer").fill("Reviewer");
  await panel.getByRole("button", { name: "Accept candidate" }).click();
  await expect(panel.getByText("Review unavailable")).toBeVisible();
  fail = false;
  await panel.getByRole("button", { name: "Accept candidate" }).click();
  await expect(
    panel.getByText(
      "Review acknowledgement did not match the selected candidate",
    ),
  ).toBeVisible();
  await expect(
    panel.getByText("accepted / Reviewer", { exact: true }),
  ).toHaveCount(0);
});

test("budget-limited reinference never validates absent candidates or enables review", async ({
  page,
}) => {
  await mockApi(page);
  let flag: "truncated" | "cancelled" | "deadline_reached" = "truncated";
  let reviewReads = 0;
  await page.route("**/v1/analysis/state-machine/**", async (route) => {
    if (new URL(route.request().url()).pathname.endsWith("/reviews")) {
      reviewReads++;
      return route.fulfill({ json: [] });
    }
    await route.fulfill({
      json: {
        api_version: "1",
        snapshot_id: snapshots[0].id,
        context_id: snapshots[0].context.id,
        analysis: {
          ...inference,
          envelope: { ...inference.envelope, [flag]: true },
        },
      },
    });
  });
  await page.goto(
    "/?snapshot=snapshot%3Atwo&context=context%3Ahost&definition=definition%3Amain&view=flow#token=test-token",
  );
  const panel = page.getByRole("region", { name: "State transition review" });
  await panel.getByLabel("Enum path").fill("State");
  await panel.getByLabel("State variable").fill("state");
  for (const next of ["truncated", "cancelled", "deadline_reached"] as const) {
    flag = next;
    await panel.getByRole("button", { name: "Analyze states" }).click();
    await expect(
      panel.getByText(
        "Review unavailable for a cancelled or truncated inference.",
      ),
    ).toBeVisible();
    await panel.getByLabel("Declared reviewer").fill("Reviewer");
    await expect(
      panel.getByRole("button", { name: "Accept candidate" }),
    ).toBeDisabled();
    expect(reviewReads).toBe(0);
  }
});
