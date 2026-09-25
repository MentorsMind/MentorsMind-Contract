// Stub file for generated staking contract bindings
// Run `pnpm run generate:staking` to generate actual bindings

export class Client {
  constructor(address: string) {
    this.address = address;
  }
  address: string;
}

export * from './types.js';
