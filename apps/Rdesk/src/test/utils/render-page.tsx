/**
 * Page-level render utilities
 *
 * Provides standardized rendering for Rdesk page tests with:
 * - Router context
 * - Shared providers (Theme, etc.)
 * - Route helpers for simulating navigation state
 */

import { render, RenderOptions } from "@testing-library/react";
import { MemoryRouter } from "react-router";

// Wrapper with common providers
interface PageProvidersProps {
  children: React.ReactNode;
  route?: string;
}

export const PageProviders = ({ children, route = "/" }: PageProvidersProps) => {
  return (
    <MemoryRouter initialEntries={[route]}>
      {children}
    </MemoryRouter>
  );
};

// Render a page at a specific route
export const renderPage = (
  ui: React.ReactElement,
  options?: {
    route?: string;
    wrapperOptions?: Omit<RenderOptions, "wrapper">;
  }
) => {
  const wrapper = (ui: React.ReactElement) => (
    <PageProviders route={options?.route}>{ui}</PageProviders>
  );

  return render(ui, { wrapper, ...options?.wrapperOptions });
};

// Re-export testing library utilities
export * from "@testing-library/react";
export { default as userEvent } from "@testing-library/user-event";
