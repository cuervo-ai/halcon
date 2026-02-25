/**
 * User Entity - Core business entity representing a Halcon CLI user
 * 
 * @domain Entity
 * @clean-architecture Core Domain
 */
export interface User {
  id: string;
  email: string;
  username: string;
  role: UserRole;
  preferences: UserPreferences;
  createdAt: Date;
  updatedAt: Date;
}

export type UserRole = 'admin' | 'developer' | 'viewer';

export interface UserPreferences {
  theme: 'light' | 'dark' | 'auto';
  terminal: TerminalPreferences;
  notifications: NotificationPreferences;
}

export interface TerminalPreferences {
  fontSize: number;
  fontFamily: string;
  colorScheme: 'halcon-dark' | 'halcon-light' | 'system';
}

export interface NotificationPreferences {
  email: boolean;
  push: boolean;
  cliUpdates: boolean;
}

/**
 * User factory for creating validated user instances
 */
export class UserFactory {
  static create(data: Partial<User>): User {
    const now = new Date();
    
    return {
      id: data.id || crypto.randomUUID(),
      email: this.validateEmail(data.email || ''),
      username: this.validateUsername(data.username || ''),
      role: data.role || 'developer',
      preferences: data.preferences || {
        theme: 'auto',
        terminal: {
          fontSize: 14,
          fontFamily: 'JetBrains Mono',
          colorScheme: 'system'
        },
        notifications: {
          email: true,
          push: false,
          cliUpdates: true
        }
      },
      createdAt: data.createdAt || now,
      updatedAt: now
    };
  }

  private static validateEmail(email: string): string {
    const emailRegex = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
    if (!emailRegex.test(email)) {
      throw new Error('Invalid email format');
    }
    return email;
  }

  private static validateUsername(username: string): string {
    if (username.length < 3 || username.length > 20) {
      throw new Error('Username must be between 3 and 20 characters');
    }
    if (!/^[a-zA-Z0-9_-]+$/.test(username)) {
      throw new Error('Username can only contain letters, numbers, underscores, and hyphens');
    }
    return username;
  }
}