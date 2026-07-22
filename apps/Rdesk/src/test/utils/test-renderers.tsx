/**
 * Test rendering utilities
 *
 * Provides custom render functions with common providers
 */

import { render, RenderOptions } from "@testing-library/react";
import { BrowserRouter } from "react-router";

// Wrapper with common providers
interface AllTheProvidersProps {
  children: React.ReactNode;
}

export const AllTheProviders = ({ children }: AllTheProvidersProps) => {
  return <BrowserRouter>{children}</BrowserRouter>;
};

// Custom render function with providers
export const renderWithProviders = (
  ui: React.ReactElement,
  options?: Omit<RenderOptions, "wrapper">
) => {
  return render(ui, { wrapper: AllTheProviders, ...options });
};

// Re-export testing library utilities
export * from "@testing-library/react";
export { default as userEvent } from "@testing-library/user-event";
