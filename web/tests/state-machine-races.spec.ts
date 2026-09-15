import {
  expect,
  test,
  type Locator,
  type Page,
  type Request,
} from "@playwright/test";
import { definitions, mockApi, snapshots } from "./fixtures";
import type {
  AnalysisResponse,
  StateMachineInference,
  StateMachineSelection,
  StateTransitionReview,
} from "../src/api/types";

type Phase = "inference" | "review read" | "review write";
type Scope = { snapshot: string; definition: string };
const initial: Scope = {
  snapshot: snapshots[0].id,
  definition: definitions[0].id,
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

function inference(scope: Scope): StateMachineInference {
  const selected = definitions.find((item) => item.id === scope.definition)!;
  const previous =
    scope.snapshot === initial.snapshot &&
    scope.definition === initial.definition;
  const label = previous ? "Previous" : "Current";
  const identity = `${scope.snapshot}/${scope.definition}`;
  return {
    envelope: {
      algorithm_version: "state-machine-race-fixture:1",
      coverage: {
        status: "partial",
        reasons: [{ reason: "syntax_only", count: 1 }],
        limitations: ["Only selected syntax candidates"],
      },
      truncated: false,
      cancelled: false,
      deadline_reached: false,
      assumptions: ["A review declaration is not a proof"],
    },
    input_digest: `input:${identity}`,
    definition_id: selected.id,
    enum_path: "State",
    state_place: "state",
    body_span: selected.span,
    visited_nodes: 20,
    candidates: [
      {
        id: `candidate:${identity}`,
        from_variant: label,
        to_variant: `${label}Done`,
        match_span: selected.span,
        arm_span: selected.span,
        assignment_span: selected.span,
        guard: null,
        action: { span: selected.span, text: `state = State::${label}Done` },
      },
    ],
    unknowns: [],
  };
}

function response(scope: Scope): AnalysisResponse<StateMachineInference> {
  return {
    api_version: "1",
    snapshot_id: scope.snapshot,
    context_id: snapshots[0].context.id,
    analysis: inference(scope),
  };
}

function declaration(scope: Scope): StateTransitionReview {
  const report = inference(scope);
  return {
    input_digest: report.input_digest,
    candidate_id: report.candidates[0].id,
    reviewer: "Earlier reviewer",
    note: "Previous selection only",
    decision: "accepted",
  };
}

function requestScope(request: Request): { scope: Scope; phase: Phase } {
  const url = new URL(request.url());
  const review = url.pathname.endsWith("/reviews");
  const phase: Phase = !review
    ? "inference"
    : request.method() === "GET"
      ? "review read"
      : "review write";
  const selection: StateMachineSelection =
    request.method() === "GET"
      ? (Object.fromEntries(url.searchParams) as StateMachineSelection)
      : review
        ? request.postDataJSON().selection
        : request.postDataJSON();
  expect(selection.context_id).toBe(snapshots[0].context.id);
  expect(selection.enum_path).toBe("State");
  expect(selection.state_place).toBe("state");
  return {
    scope: {
      snapshot: selection.snapshot_id,
      definition: decodeURIComponent(url.pathname.split("/")[4]),
    },
    phase,
  };
}

async function open(page: Page) {
  await page.goto(
    "/?snapshot=snapshot%3Atwo&context=context%3Ahost&definition=definition%3Amain&view=flow#token=test-token",
  );
  const panel = page.getByRole("region", { name: "State transition review" });
  await expect(panel).toBeVisible();
  return panel;
}

async function analyze(panel: Locator) {
  await panel.getByLabel("Enum path").fill("State");
  await panel.getByLabel("State variable").fill("state");
  await panel.getByRole("button", { name: "Analyze states" }).click();
}

for (const phase of ["inference", "review read", "review write"] as const)
  for (const navigation of ["definition", "snapshot"] as const)
    test(`late ${phase} is discarded after ${navigation} navigation`, async ({
      page,
    }) => {
      await mockApi(page);
      const arrived = deferred<Request>();
      const release = deferred<void>();
      const finished = deferred<void>();
      const failures: Request[] = [];
      const errors: Error[] = [];
      const calls: { scope: Scope; phase: Phase }[] = [];
      page.on("requestfailed", (request) => failures.push(request));
      page.on("pageerror", (error) => errors.push(error));
      let held = false;
      await page.route("**/v1/analysis/state-machine/**", async (route) => {
        const call = requestScope(route.request());
        calls.push(call);
        const previous =
          call.scope.snapshot === initial.snapshot &&
          call.scope.definition === initial.definition;
        const delayed = previous && call.phase === phase && !held;
        const body =
          call.phase === "inference"
            ? response(call.scope)
            : call.phase === "review write"
              ? route.request().postDataJSON().review
              : delayed
                ? [declaration(call.scope)]
                : [];
        if (delayed) {
          held = true;
          arrived.resolve(route.request());
          await release.promise;
        }
        try {
          await route.fulfill({ json: body });
        } finally {
          if (delayed) finished.resolve();
        }
      });
      const panel = await open(page);
      try {
        await analyze(panel);
        if (phase === "review write") {
          await panel.getByLabel("Declared reviewer").fill("Earlier reviewer");
          await panel.getByLabel("Review note").fill("Previous selection only");
          await panel.getByRole("button", { name: "Accept candidate" }).click();
        }
        const pending = await arrived.promise;
        if (phase === "inference") {
          await expect(
            panel.getByText("Inferring selected state transitions", {
              exact: true,
            }),
          ).toBeVisible();
          await expect(
            panel.getByRole("button", { name: "Accept candidate" }),
          ).toHaveCount(0);
        } else {
          await expect(
            panel.getByRole("button", { name: "Accept candidate" }),
          ).toBeDisabled();
          await expect(
            panel.getByRole("button", { name: "Reject candidate" }),
          ).toBeDisabled();
        }

        const next =
          navigation === "definition"
            ? { snapshot: initial.snapshot, definition: definitions[1].id }
            : { snapshot: snapshots[1].id, definition: initial.definition };
        if (navigation === "definition")
          await page.getByTitle("demo::handle", { exact: true }).click();
        else
          await page
            .getByLabel("Snapshot", { exact: true })
            .selectOption(next.snapshot);
        await expect(page).toHaveURL(
          (url) =>
            url.searchParams.get("snapshot") === next.snapshot &&
            url.searchParams.get("definition") === next.definition &&
            url.searchParams.get("view") === "flow",
        );
        await expect(panel.getByLabel("Enum path")).toHaveValue("");
        await expect(
          panel.getByRole("button", { name: "Accept candidate" }),
        ).toHaveCount(0);
        await expect(
          panel.getByText("Inferring selected state transitions", {
            exact: true,
          }),
        ).toHaveCount(0);
        expect(
          calls.filter(
            (call) =>
              call.scope.snapshot === next.snapshot &&
              call.scope.definition === next.definition,
          ),
        ).toHaveLength(0);

        await analyze(panel);
        await expect(
          panel.getByRole("button", { name: "Current to CurrentDone" }),
        ).toBeVisible();
        await panel.getByLabel("Declared reviewer").fill("Current reviewer");
        await expect(
          panel.getByRole("button", { name: "Accept candidate" }),
        ).toBeEnabled();
        release.resolve();
        await finished.promise;
        await expect.poll(() => failures.includes(pending)).toBe(true);
        await page.evaluate(
          () =>
            new Promise<void>((resolve) =>
              requestAnimationFrame(() =>
                requestAnimationFrame(() => resolve()),
              ),
            ),
        );
        await expect(
          panel.getByRole("button", { name: "Previous to PreviousDone" }),
        ).toHaveCount(0);
        await expect(panel.getByText(/Earlier reviewer/)).toHaveCount(0);
        await expect(
          panel.getByRole("button", { name: "Current to CurrentDone" }),
        ).toBeVisible();
        await expect(
          panel.getByRole("button", { name: "Accept candidate" }),
        ).toBeEnabled();
        await expect(
          panel.getByRole("button", { name: "Reject candidate" }),
        ).toBeEnabled();
        expect(
          calls.filter((call) => call.phase === "review write"),
        ).toHaveLength(phase === "review write" ? 1 : 0);
        expect(errors).toEqual([]);
      } finally {
        release.resolve();
        if (held) await finished.promise;
      }
    });

test("review-list loading and failure disable both decisions until explicit successful retry", async ({
  page,
}) => {
  await mockApi(page);
  const arrived = deferred<void>();
  const release = deferred<void>();
  const finished = deferred<void>();
  let reads = 0;
  let writes = 0;
  await page.route("**/v1/analysis/state-machine/**", async (route) => {
    const call = requestScope(route.request());
    if (call.phase === "inference")
      return route.fulfill({ json: response(call.scope) });
    if (call.phase === "review write") {
      writes++;
      return route.fulfill({ json: route.request().postDataJSON().review });
    }
    if (++reads !== 1) return route.fulfill({ json: [] });
    arrived.resolve();
    await release.promise;
    try {
      await route.fulfill({
        status: 503,
        json: {
          code: "unavailable_evidence",
          message: "Reviewer declarations unavailable",
        },
      });
    } finally {
      finished.resolve();
    }
  });
  const panel = await open(page);
  try {
    await analyze(panel);
    await arrived.promise;
    await panel.getByLabel("Declared reviewer").fill("Current reviewer");
    await expect(
      panel.getByText("Loading reviewer declarations", { exact: true }),
    ).toBeVisible();
    for (const name of ["Accept candidate", "Reject candidate"])
      await expect(panel.getByRole("button", { name })).toBeDisabled();
    release.resolve();
    await finished.promise;
    await expect(
      panel.getByText("Reviewer declarations unavailable", { exact: true }),
    ).toBeVisible();
    await expect(
      panel.getByText("Loading reviewer declarations", { exact: true }),
    ).toHaveCount(0);
    for (const name of ["Accept candidate", "Reject candidate"])
      await expect(panel.getByRole("button", { name })).toBeDisabled();
    expect(writes).toBe(0);
    await panel.getByRole("button", { name: "Analyze states" }).click();
    for (const name of ["Accept candidate", "Reject candidate"])
      await expect(panel.getByRole("button", { name })).toBeEnabled();
    await expect(
      panel.getByText("Reviewer declarations unavailable", { exact: true }),
    ).toHaveCount(0);
    expect(reads).toBe(2);
    expect(writes).toBe(0);
  } finally {
    release.resolve();
    if (reads > 0) await finished.promise;
  }
});
