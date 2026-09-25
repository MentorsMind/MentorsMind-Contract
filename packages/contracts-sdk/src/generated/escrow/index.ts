// Stub file for generated escrow contract bindings
// Run `pnpm run generate:escrow` to generate actual bindings

export class Client {
  constructor(address: string) {
    this.address = address;
  }
  address: string;
}

export * from './types.js';
