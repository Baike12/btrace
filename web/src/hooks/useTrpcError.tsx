// PG-only: TRPCClientError replaced with generic
const TRPCClientError = { from: (_e: unknown) => new Error("API Error") };

export function useTrpcError(
  error: unknown | null,
  silentHttpCodes: number[],
): { isSilentError: boolean } {
  return {
    isSilentError:
      error instanceof TRPCClientError &&
      typeof error.data?.httpStatus === "number" &&
      silentHttpCodes.includes(error.data?.httpStatus),
  };
}
