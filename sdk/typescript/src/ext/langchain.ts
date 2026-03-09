/**
 * LangChain.js integration for Mnemo.
 *
 * Provides a `MnemoChatMessageHistory` that stores chat history
 * in the Mnemo memory API, enabling cross-session persistence
 * and semantic retrieval of past conversations.
 *
 * @example
 * ```ts
 * import { MnemoChatMessageHistory } from 'mnemo-client/langchain';
 * import { ChatOpenAI } from '@langchain/openai';
 * import { RunnableWithMessageHistory } from '@langchain/core/runnables';
 *
 * const history = new MnemoChatMessageHistory({
 *   baseUrl: 'http://localhost:8080',
 *   user: 'alice',
 *   sessionId: 'session-1',
 * });
 * ```
 */

import type { BaseMessage } from '@langchain/core/messages';
import { HumanMessage, AIMessage, SystemMessage } from '@langchain/core/messages';
import { BaseListChatMessageHistory } from '@langchain/core/chat_history';
import { MnemoClient } from '../client.js';
import type { MnemoClientOptions } from '../types.js';

export interface MnemoChatMessageHistoryOptions extends MnemoClientOptions {
  /** Mnemo user identifier. */
  user: string;
  /** Session ID for grouping messages. If omitted, a new session is created. */
  sessionId?: string;
}

export class MnemoChatMessageHistory extends BaseListChatMessageHistory {
  lc_namespace = ['mnemo', 'chat_history'];

  private client: MnemoClient;
  private user: string;
  private sessionId?: string;

  constructor(options: MnemoChatMessageHistoryOptions) {
    super();
    this.client = new MnemoClient(options);
    this.user = options.user;
    this.sessionId = options.sessionId;
  }

  async getMessages(): Promise<BaseMessage[]> {
    if (!this.sessionId) return [];
    // Use the session messages endpoint for chronological message retrieval
    const result = await this.client.getMessages(this.sessionId, { limit: 200 });
    return (result.messages || []).map((msg) => {
      const content = String(msg.content ?? '');
      const role = String(msg.role ?? 'user');
      if (role === 'assistant') return new AIMessage(content);
      if (role === 'system') return new SystemMessage(content);
      return new HumanMessage(content);
    });
  }

  async addMessage(message: BaseMessage): Promise<void> {
    const role = message._getType() === 'ai' ? 'assistant' : message._getType() === 'system' ? 'system' : 'user';
    const result = await this.client.add(this.user, String(message.content), {
      role: role as 'user' | 'assistant' | 'system',
      sessionId: this.sessionId,
    });
    if (!this.sessionId) {
      this.sessionId = result.session_id;
    }
  }

  async addMessages(messages: BaseMessage[]): Promise<void> {
    for (const msg of messages) {
      await this.addMessage(msg);
    }
  }

  async clear(): Promise<void> {
    if (!this.sessionId) return;
    await this.client.clearMessages(this.sessionId);
  }
}
