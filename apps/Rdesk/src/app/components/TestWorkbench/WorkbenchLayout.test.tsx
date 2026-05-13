import { render, screen } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router";
import { describe, expect, it } from "vitest";
import { getMockInvoke } from "../../../test/mocks/tauri";
import { WorkbenchLayout } from "./WorkbenchLayout";

function renderWorkbench() {
  getMockInvoke().mockImplementation((command: string) => {
    if (command === "get_system_resource_snapshot") {
      return Promise.resolve(null);
    }
    return Promise.resolve(null);
  });

  return render(
    <MemoryRouter initialEntries={["/test/matrix"]}>
      <Routes>
        <Route path="/test" element={<WorkbenchLayout />}>
          <Route path="matrix" element={<div>Matrix page content</div>} />
        </Route>
      </Routes>
    </MemoryRouter>
  );
}

describe("WorkbenchLayout scrolling", () => {
  it("keeps the workbench chrome fixed and limits scrolling to inner panes", () => {
    const { container } = renderWorkbench();
    const root = container.firstElementChild;

    expect(root).toHaveClass("overflow-hidden");
    expect(screen.getByRole("navigation")).toHaveClass("overflow-y-auto");
    expect(screen.getByRole("main")).toHaveClass("overflow-auto");
  });
});
