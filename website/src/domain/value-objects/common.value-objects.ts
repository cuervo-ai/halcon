/**
 * Email Value Object - Immutable email representation
 * 
 * @domain Value Object
 * @clean-architecture Core Domain
 */
export class Email {
  private readonly value: string;

  private constructor(email: string) {
    this.value = email;
    this.validate();
  }

  static create(email: string): Email {
    return new Email(email);
  }

  private validate(): void {
    const emailRegex = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
    if (!emailRegex.test(this.value)) {
      throw new Error('Invalid email format');
    }
  }

  getValue(): string {
    return this.value;
  }

  getDomain(): string {
    return this.value.split('@')[1];
  }

  getUsername(): string {
    return this.value.split('@')[0];
  }

  equals(other: Email): boolean {
    return this.value.toLowerCase() === other.value.toLowerCase();
  }

  toString(): string {
    return this.value;
  }
}

/**
 * URL Value Object - Immutable URL representation
 */
export class Url {
  private readonly value: string;

  private constructor(url: string) {
    this.value = url;
    this.validate();
  }

  static create(url: string): Url {
    return new Url(url);
  }

  private validate(): void {
    try {
      new URL(this.value);
    } catch {
      throw new Error('Invalid URL format');
    }
  }

  getValue(): string {
    return this.value;
  }

  getProtocol(): string {
    return new URL(this.value).protocol.replace(':', '');
  }

  getHostname(): string {
    return new URL(this.value).hostname;
  }

  getPath(): string {
    return new URL(this.value).pathname;
  }

  equals(other: Url): boolean {
    return this.value === other.value;
  }

  toString(): string {
    return this.value;
  }
}

/**
 * Semantic Version Value Object
 */
export class SemanticVersion {
  readonly major: number;
  readonly minor: number;
  readonly patch: number;
  readonly prerelease?: string;
  readonly build?: string;

  constructor(version: string) {
    const parsed = this.parse(version);
    this.major = parsed.major;
    this.minor = parsed.minor;
    this.patch = parsed.patch;
    this.prerelease = parsed.prerelease;
    this.build = parsed.build;
  }

  private parse(version: string) {
    const regex = /^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/;
    const match = version.match(regex);

    if (!match) {
      throw new Error('Invalid semantic version format');
    }

    return {
      major: parseInt(match[1], 10),
      minor: parseInt(match[2], 10),
      patch: parseInt(match[3], 10),
      prerelease: match[4],
      build: match[5]
    };
  }

  compare(other: SemanticVersion): number {
    if (this.major !== other.major) return this.major - other.major;
    if (this.minor !== other.minor) return this.minor - other.minor;
    if (this.patch !== other.patch) return this.patch - other.patch;
    return 0;
  }

  isGreaterThan(other: SemanticVersion): boolean {
    return this.compare(other) > 0;
  }

  isLessThan(other: SemanticVersion): boolean {
    return this.compare(other) < 0;
  }

  equals(other: SemanticVersion): boolean {
    return this.compare(other) === 0;
  }

  toString(): string {
    let result = `${this.major}.${this.minor}.${this.patch}`;
    if (this.prerelease) result += `-${this.prerelease}`;
    if (this.build) result += `+${this.build}`;
    return result;
  }
}