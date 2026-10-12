<script lang="ts">
  import { onDestroy } from 'svelte';
  import { createChat } from './lib/chat.svelte.js';
  import { groupItems, type Group } from './lib/group';

  const scheme = location.protocol === 'https:' ? 'wss' : 'ws';
  const base = import.meta.env.BASE_URL.replace(/\/$/, '');
  const chat = createChat(`${scheme}://${location.host}${base}/ws`);
  onDestroy(chat.close);

  let draft = $state('');
  let input = $state<HTMLTextAreaElement>();
  let list = $state<HTMLDivElement>();
  let stickToBottom = true;

  const groups = $derived(groupItems(chat.items, chat.me));

  const busy = $derived(
    chat.items.some(
      (i) =>
        (i.kind === 'reply' && i.state === 'streaming') ||
        (i.kind === 'prompt' && (i.state === 'queued' || i.state === 'answering')),
    ),
  );

  function variant(group: Group): 'own' | 'assistant' | 'other' {
    if (group.own) return 'own';
    return group.key === 'assistant' ? 'assistant' : 'other';
  }

  function send() {
    const text = draft.trim();
    if (text && chat.submit(text)) draft = '';
  }

  function onKeydown(event: KeyboardEvent) {
    const modifier = event.shiftKey || event.ctrlKey || event.metaKey;
    if (event.key === 'Enter' && !modifier && !event.isComposing) {
      event.preventDefault();
      send();
    }
  }

  // Follow new messages, unless the reader has scrolled up to look at older ones.
  function onScroll() {
    if (!list) return;
    stickToBottom = list.scrollHeight - list.scrollTop - list.clientHeight < 80;
  }

  $effect(() => {
    void chat.items;
    if (stickToBottom) list?.scrollTo({ top: list.scrollHeight });
  });

  // Grow the textarea with its content.
  $effect(() => {
    void draft;
    if (!input) return;
    input.style.height = 'auto';
    input.style.height = `${input.scrollHeight + 2}px`;
  });
</script>

<div class="chat">
  <header class="chat__header">
    <h1>Multi-user Chat</h1>
    <div class="chat__status">
      {#if chat.status !== 'open'}
        <span class="pill pill--warn">
          {chat.status === 'connecting' ? 'Connecting…' : 'Reconnecting…'}
        </span>
      {:else if busy}
        <span class="pill pill--busy"><span class="dot"></span>AI cooking</span>
      {/if}
    </div>
  </header>

  <div class="chat__messages" bind:this={list} onscroll={onScroll}>
    {#each groups as group (group.items[0].key)}
      <section class="group group--{variant(group)}">
        {#if variant(group) !== 'assistant'}
          <div class="group__name">{group.label}</div>
        {/if}

        {#each group.items as item (item.key)}
          {#if item.kind === 'prompt'}
            <p class="bubble" class:is-queued={item.state === 'queued'}
               class:is-dropped={item.state === 'withdrawn' || item.state === 'rejected'}>
              {item.content}
              {#if item.state === 'withdrawn' || item.state === 'rejected'}
                <small class="bubble__note">{item.state}</small>
              {/if}
            </p>
          {:else}
            <p class="bubble">
              {#if item.state === 'streaming' && item.content === ''}
                <span class="typing" aria-label="Assistant is typing"><span></span><span></span><span></span></span>
              {:else}
                {item.content}
              {/if}
              {#if item.state === 'cancelled' || item.state === 'failed'}
                <small class="bubble__note">{item.state}</small>
              {/if}
            </p>
          {/if}
        {/each}
      </section>
    {/each}
  </div>

  <form class="chat__composer" onsubmit={(e) => { e.preventDefault(); send(); }}>
    <textarea
      bind:this={input}
      bind:value={draft}
      onkeydown={onKeydown}
      rows="1"
      placeholder="Say something..."
      aria-label="Message"
      autocomplete="off"
    ></textarea>
    <button type="submit" disabled={!draft.trim()}>Send</button>
  </form>
</div>
