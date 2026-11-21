<!-- A page's title over grey bars while its data loads. -->
<script lang="ts">
  let { title }: { title: string } = $props();
  const widths = [0, 1, 2, 3].map(() => 40 + Math.random() * 40);
</script>

<div class="page">
  <div class="page-head"><div><h1>{title}</h1></div></div>
  <div class="stack">{#each widths as w}<div class="skeleton" style:width="{w}%"></div>{/each}</div>
</div>
