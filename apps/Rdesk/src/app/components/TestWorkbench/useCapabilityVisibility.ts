import { useCallback, useEffect, useState } from "react";
import type { CapabilityStatus } from "../../services/capabilityMatrix";

const STORAGE_KEY = "rdesk.testWorkbench.showUnavailableCapabilities";
const CHANGE_EVENT = "rdesk-test-workbench-show-unavailable-changed";

export function readShowUnavailableCapabilities(): boolean {
  if (typeof window === "undefined") return false;
  return window.localStorage?.getItem(STORAGE_KEY) === "true";
}

function writeShowUnavailableCapabilities(value: boolean) {
  window.localStorage?.setItem(STORAGE_KEY, value ? "true" : "false");
  window.dispatchEvent(new CustomEvent<boolean>(CHANGE_EVENT, { detail: value }));
}

export function useShowUnavailableCapabilities() {
  const [showUnavailable, setShowUnavailableState] = useState(readShowUnavailableCapabilities);

  useEffect(() => {
    const handleChange = (event: Event) => {
      if (event instanceof CustomEvent && typeof event.detail === "boolean") {
        setShowUnavailableState(event.detail);
        return;
      }
      setShowUnavailableState(readShowUnavailableCapabilities());
    };

    window.addEventListener(CHANGE_EVENT, handleChange);
    window.addEventListener("storage", handleChange);
    return () => {
      window.removeEventListener(CHANGE_EVENT, handleChange);
      window.removeEventListener("storage", handleChange);
    };
  }, []);

  const setShowUnavailable = useCallback((value: boolean) => {
    setShowUnavailableState(value);
    writeShowUnavailableCapabilities(value);
  }, []);

  return [showUnavailable, setShowUnavailable] as const;
}

export function shouldShowCapabilityOption(
  available: boolean,
  showUnavailable: boolean
): boolean {
  return showUnavailable || available;
}

export function shouldShowCapabilityStatus(
  status: CapabilityStatus,
  showUnavailable: boolean
): boolean {
  if (showUnavailable) return true;
  return status === "available" || status === "usable" || status === "degraded";
}
