<script lang="ts">
  let { title, error }: { title: string; error: string } = $props();
</script>

<div class="page">
  <div class="page-head"><div><h1>{title}</h1></div></div>
  <div class="callout">{error}</div>
</div>
