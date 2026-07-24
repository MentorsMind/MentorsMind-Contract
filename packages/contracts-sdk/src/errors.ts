/**
 * Common Soroban contract error codes or generic application level typed union for
 * contract panic messages.
 */
export type ContractPanicReason =
    | 'AuthError'
    | 'InvalidAmount'
    | 'NotFound'
    | 'AlreadyInitialized'
    | 'Unauthorized'
    | 'InsufficientBalance'
    | 'Expired'
    | 'UnknownError';

export class ContractError extends Error {
    public reason: ContractPanicReason;

    constructor(reason: ContractPanicReason, message?: string) {
        super(message || `Contract reverted with reason: ${reason}`);
        this.name = 'ContractError';
        this.reason = reason;
    }
}
