// @vitest-environment jsdom
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import userEvent from "@testing-library/user-event";
import { DeadlinePicker, formatDeadline, parseTime } from "./DeadlinePicker";

describe("deadline picker", () => {
  afterEach(cleanup);
  it("parses typed times", () => {
    expect(parseTime("13:00")).toEqual([13, 0]);
    expect(parseTime("1 pm")).toEqual([13, 0]);
    expect(parseTime("1:30pm")).toEqual([13, 30]);
    expect(parseTime("12 am")).toEqual([0, 0]);
    expect(parseTime("9.15")).toEqual([9, 15]);
    expect(parseTime("25:00")).toBeNull();
    expect(parseTime("abc")).toBeNull();
  });
  it("shows the date only when no time was chosen", () => {
    expect(formatDeadline("2026-10-31T23:59")).not.toMatch(/59/);
    expect(formatDeadline("2026-10-31T16:30")).toMatch(/4:30|16:30/);
  });
  it("picks a day and types a time without losing either", () => {
    const onChange = vi.fn();
    const { rerender } = render(
      <DeadlinePicker value="" onChange={onChange} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "No deadline" }));
    const now = new Date();
    const day = new Date(now.getFullYear(), now.getMonth(), 15);
    fireEvent.click(screen.getByRole("button", { name: day.toDateString() }));
    expect(onChange).toHaveBeenLastCalledWith(
      expect.stringMatching(/-15T23:59$/),
    );
    rerender(
      <DeadlinePicker value={onChange.mock.lastCall![0]} onChange={onChange} />,
    );
    const time = screen.getByLabelText("Deadline time");
    fireEvent.change(time, { target: { value: "1:30 pm" } });
    fireEvent.keyDown(time, { key: "Enter" });
    expect(onChange).toHaveBeenLastCalledWith(
      expect.stringMatching(/-15T13:30$/),
    );
  });
});

it("sets a deadline from digits without a manually typed separator", async () => {
  const user = userEvent.setup();
  const change = vi.fn();
  render(<DeadlinePicker value="2030-10-22T23:59" onChange={change} />);
  await user.click(
    screen.getByRole("button", {
      name: formatDeadline("2030-10-22T23:59"),
    }),
  );
  await user.click(screen.getByLabelText("Deadline time"));
  await user.keyboard("0930{Enter}");
  expect(change).toHaveBeenLastCalledWith("2030-10-22T09:30");
});
