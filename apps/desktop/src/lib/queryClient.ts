import { QueryClient } from "@tanstack/react-query";

/// App-wide query client. Tuned for a local desktop app talking to an on-device
/// SQLite catalog: data only changes on an explicit refresh/import, so we cache
/// aggressively and never refetch on window focus.
export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 5 * 60 * 1000, // 5 min — catalog is static between refreshes
      gcTime: 30 * 60 * 1000,
      refetchOnWindowFocus: false,
      retry: 1,
    },
  },
});
