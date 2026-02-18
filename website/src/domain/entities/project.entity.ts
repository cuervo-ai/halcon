/**
 * Project Entity - Represents a development project using Halcon CLI
 * 
 * @domain Entity
 * @clean-architecture Core Domain
 */
export interface Project {
  id: string;
  name: string;
  slug: string;
  description: string;
  repositoryUrl?: string;
  technology: TechnologyStack;
  status: ProjectStatus;
  createdAt: Date;
  updatedAt: Date;
  lastActivityAt: Date;
}

export type ProjectStatus = 'active' | 'archived' | 'template';

export interface TechnologyStack {
  language: ProgrammingLanguage;
  framework?: string;
  packageManager: PackageManager;
  version: string;
}

export type ProgrammingLanguage = 'rust' | 'typescript' | 'javascript' | 'python' | 'go' | 'java' | 'other';
export type PackageManager = 'cargo' | 'npm' | 'yarn' | 'pnpm' | 'pip' | 'go-mod' | 'maven' | 'other';

/**
 * Project factory for creating validated project instances
 */
export class ProjectFactory {
  static create(data: Partial<Project>): Project {
    const now = new Date();
    
    return {
      id: data.id || crypto.randomUUID(),
      name: this.validateName(data.name || ''),
      slug: this.generateSlug(data.name || ''),
      description: data.description || '',
      repositoryUrl: data.repositoryUrl,
      technology: data.technology || {
        language: 'typescript',
        packageManager: 'npm',
        version: '1.0.0'
      },
      status: data.status || 'active',
      createdAt: data.createdAt || now,
      updatedAt: now,
      lastActivityAt: now
    };
  }

  private static validateName(name: string): string {
    if (name.length < 3 || name.length > 50) {
      throw new Error('Project name must be between 3 and 50 characters');
    }
    return name;
  }

  private static generateSlug(name: string): string {
    return name
      .toLowerCase()
      .replace(/[^\w\s-]/g, '')
      .replace(/\s+/g, '-')
      .replace(/--+/g, '-')
      .trim();
  }

  static updateActivity(project: Project): Project {
    return {
      ...project,
      updatedAt: new Date(),
      lastActivityAt: new Date()
    };
  }
}