<script lang="ts">
  import { STATUS } from "../lib/format";
  let { status }: { status: string } = $props();
</script>

<span class="pill {status}">{STATUS[status] || status}</span>
