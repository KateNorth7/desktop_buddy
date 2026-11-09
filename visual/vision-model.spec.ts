import { expect, test } from "@playwright/test";

test("AI settings display the resolved screenshot models without saving an override", async ({
  page,
}, info) => {
  await page.goto(
    `/visual/index.html?theme=${info.project.name.endsWith("dark") ? "dark" : "light"}`,
  );
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page
    .getByRole("navigation", { name: "Settings sections" })
    .getByRole("button", { name: "AI", exact: true })
    .click();
  const services = page.getByRole("region", { name: "Services", exact: true });
  const row = services.getByRole("group", {
    name: "Vision model",
    exact: true,
  });
  await expect(
    row.getByText("screenshots · claude-haiku-5-5", { exact: true }),
  ).toBeVisible();
  await expect(row.getByText("Uses Claude", { exact: true })).toBeVisible();
  await expect(
    row.getByRole("button", { name: "Change model", exact: true }),
  ).toBeVisible();
  await expect(
    services.getByRole("textbox", { name: "Vision model ID", exact: true }),
  ).toHaveCount(0);
  const bounds = await row.evaluate((item) => ({
    left: item.getBoundingClientRect().left,
    right: item.getBoundingClientRect().right,
  }));
  expect(bounds.right).toBeLessThanOrEqual(page.viewportSize()!.width);
  const label = await row.locator(".svc b").evaluate((element) => {
    const style = getComputedStyle(element);
    return {
      height: element.getBoundingClientRect().height,
      line: parseFloat(style.lineHeight),
    };
  });
  expect(label.height).toBeLessThanOrEqual(label.line + 1);
  await row.getByRole("button", { name: "Change model", exact: true }).click();
  const field = services.getByRole("textbox", {
    name: "Vision model ID",
    exact: true,
  });
  await expect(field).toHaveValue("claude-haiku-5-5");
  await field.fill("cancelled-model");
  await row.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(field).toHaveCount(0);
  const calls = await page.evaluate(
    () =>
      (window as unknown as { __visual: { calls: string[] } }).__visual.calls,
  );
  expect(calls).not.toContain("set_core_preferences");
  await page.screenshot({ path: info.outputPath("vision-model.png") });
});
